use crate::{BoundSessionState, ControlMessage, NodeConfig, RuntimeStatus, StatusMessage};
use anyhow::Context;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use once_cell::sync::Lazy;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceRegistration {
    pub id: u64,
    pub device_id: Option<String>,
    pub client_id: Option<String>,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub version: Option<String>,
    pub token: Option<String>,
    pub status: Option<String>,
    pub runtime_mode: Option<String>,
    pub latency_ms: Option<u64>,
    pub bound_session_id: Option<i64>,
    pub bound_session_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRequestEvent {
    pub request_id: String,
    pub action: String,
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    data: T,
}

#[derive(Debug, Clone)]
pub enum RuntimeUpdate {
    Registered(DeviceRegistration),
    Status(RuntimeStatus),
    Error(String),
    ActionRequest(ActionRequestEvent),
}

static OUTBOUND_TX: Lazy<Mutex<Option<mpsc::UnboundedSender<Value>>>> =
    Lazy::new(|| Mutex::new(None));

pub async fn ensure_registration(config: &mut NodeConfig) -> anyhow::Result<DeviceRegistration> {
    let server_url = config.server_url.trim();
    let api_token = config.token.trim();
    if server_url.is_empty() || api_token.is_empty() {
        anyhow::bail!("server_url and token are required before registration");
    }

    let client = Client::builder().build().context("failed to build reqwest client")?;
    let payload = json!({
        "device_id": config.device_id,
        "name": config.client_name,
        "platform": current_platform_name(),
        "version": env!("CARGO_PKG_VERSION"),
        "status": "offline",
        "capabilities": [
            "browser.launch","browser.goto","browser.read","browser.extract","browser.click","browser.type","browser.screenshot",
            "file.list","file.stat","file.read_text","file.open","screen.capture","system.info"
        ],
        "settings": {
            "runtime_mode": current_runtime_mode(),
            "browser_preference": config.browser_preference,
            "browser_mode": config.browser_mode
        }
    });

    let url = format!("{}/agent/v1/devices/register", server_url.trim_end_matches('/'));
    let response = client
        .post(url)
        .bearer_auth(api_token)
        .json(&payload)
        .send()
        .await
        .context("device register request failed")?;
    let response =
        response.error_for_status().context("device register response not successful")?;
    let envelope = response
        .json::<ApiEnvelope<DeviceRegistration>>()
        .await
        .context("failed to decode device register response")?;

    if let Some(client_id) =
        envelope.data.client_id.clone().filter(|value| !value.trim().is_empty())
    {
        config.client_id = client_id;
    }
    if let Some(name) = envelope.data.name.clone().filter(|value| !value.trim().is_empty()) {
        config.client_name = name;
    }

    Ok(envelope.data)
}

pub async fn run_runtime(config: NodeConfig) -> anyhow::Result<()> {
    run_runtime_with_updates(config, None).await
}

pub async fn run_runtime_with_updates(
    mut config: NodeConfig,
    updates: Option<mpsc::UnboundedSender<RuntimeUpdate>>,
) -> anyhow::Result<()> {
    let registered = ensure_registration(&mut config).await?;
    info!(client_id = ?registered.client_id, device_id = ?registered.device_id, "node registered");
    println!(
        "[node] registered device_id={:?} client_id={:?}",
        registered.device_id, registered.client_id
    );
    *OUTBOUND_TX.lock().expect("outbound lock") = None;

    if let Some(tx) = &updates {
        let _ = tx.send(RuntimeUpdate::Registered(registered));
    }

    let (socket, ws_url) = connect_ws(&config).await?;
    info!(ws_url = %ws_url, "connected node websocket");
    println!("[node] connected websocket {}", ws_url);
    let (mut writer, mut reader) = socket.split();

    publish_status(&mut writer, &config, None).await?;

    let initial_snapshot = fetch_device_snapshot(&config).await.ok();
    if let Some(tx) = &updates {
        let _ = tx.send(RuntimeUpdate::Status(status_from_snapshot(
            &config,
            initial_snapshot.as_ref(),
            None,
        )));
    }

    let mut ping_timer = interval(Duration::from_secs(20));
    let mut status_timer = interval(Duration::from_secs(30));
    let mut last_ping_at: Option<Instant> = None;
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Value>();
    *OUTBOUND_TX.lock().expect("outbound lock") = Some(outbound_tx);

    loop {
        tokio::select! {
            _ = ping_timer.tick() => {
                let ping = json!({
                    "type": "ping",
                    "id": format!("ping-{}", uuid::Uuid::now_v7())
                });
                writer.send(Message::Text(ping.to_string())).await.context("failed to send ping")?;
                last_ping_at = Some(Instant::now());
            }
            _ = status_timer.tick() => {
                let snapshot = fetch_device_snapshot(&config).await.ok();
                let latency = snapshot.as_ref().and_then(|item| item.latency_ms);
                publish_status(&mut writer, &config, latency).await?;
                if let Some(tx) = &updates {
                    let _ = tx.send(RuntimeUpdate::Status(status_from_snapshot(&config, snapshot.as_ref(), latency)));
                }
            }
            outbound = outbound_rx.recv() => {
                if let Some(outbound) = outbound {
                    writer.send(Message::Text(outbound.to_string())).await.context("failed to send outbound websocket message")?;
                }
            }
            incoming = reader.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        handle_ws_text(&text, &config, &updates, &mut last_ping_at).await?;
                    }
                    Some(Ok(Message::Ping(bytes))) => {
                        writer.send(Message::Pong(bytes)).await.ok();
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        if let Some(tx) = &updates {
                            let _ = tx.send(RuntimeUpdate::Error(error.to_string()));
                        }
                        return Err(error).context("websocket read failed")
                    }
                    None => break,
                }
            }
        }
    }

    if let Some(tx) = &updates {
        let snapshot = fetch_device_snapshot(&config).await.ok();
        let mut status = status_from_snapshot(&config, snapshot.as_ref(), None);
        status.connected = false;
        status.latency_ms = None;
        let _ = tx.send(RuntimeUpdate::Status(status));
    }

    Ok(())
}

pub async fn publish_action_result(
    _config: &NodeConfig,
    request_id: &str,
    status: &str,
    result: Value,
    artifacts: Vec<Value>,
    error: Option<String>,
) -> anyhow::Result<()> {
    let artifacts = publish_artifact_chunks(request_id, artifacts)?;
    let envelope = json!({
        "type": "publish",
        "id": format!("result-{}", uuid::Uuid::now_v7()),
        "topic": "ai.node.action.result",
        "payload": {
            "request_id": request_id,
            "status": status,
            "result": result,
            "artifacts": artifacts,
            "error": error.unwrap_or_default(),
            "meta": {}
        }
    });
    if let Some(tx) = OUTBOUND_TX.lock().expect("outbound lock").as_ref() {
        let _ = tx.send(envelope);
        println!("[node] queued action result {}", request_id);
        return Ok(());
    }
    anyhow::bail!("runtime outbound channel unavailable")
}

fn publish_artifact_chunks(request_id: &str, artifacts: Vec<Value>) -> anyhow::Result<Vec<Value>> {
    let mut normalized = Vec::new();
    for artifact in artifacts {
        let Some(object) = artifact.as_object() else {
            continue;
        };
        let data_url = object.get("url").and_then(Value::as_str).unwrap_or("");
        if !data_url.starts_with("data:") {
            normalized.push(artifact);
            continue;
        }
        let Some((header, encoded)) = data_url.split_once(',') else {
            normalized.push(artifact);
            continue;
        };
        let mime_type = header
            .strip_prefix("data:")
            .and_then(|value| value.strip_suffix(";base64"))
            .unwrap_or("application/octet-stream")
            .to_string();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .context("failed to decode artifact data url")?;
        let artifact_id = format!("artifact-{}", uuid::Uuid::now_v7());
        let encoded_all = base64::engine::general_purpose::STANDARD.encode(decoded);
        let chunk_size = 128 * 1024;
        let chunk_total = encoded_all.len().div_ceil(chunk_size);
        if let Some(tx) = OUTBOUND_TX.lock().expect("outbound lock").as_ref() {
            for (index, chunk) in encoded_all.as_bytes().chunks(chunk_size).enumerate() {
                let envelope = json!({
                    "type": "publish",
                    "id": format!("artifact-{}-{}", artifact_id, index),
                    "topic": "ai.node.action.artifact",
                    "payload": {
                        "request_id": request_id,
                        "artifact_id": artifact_id,
                        "artifact_type": object.get("type").cloned().unwrap_or_else(|| json!("file")),
                        "mime_type": mime_type,
                        "filename": object.get("filename").cloned().unwrap_or_else(|| json!("artifact")),
                        "bytes": object.get("bytes").cloned().unwrap_or_else(|| json!(encoded_all.len())),
                        "width": object.get("width").cloned().unwrap_or_else(|| json!(0)),
                        "height": object.get("height").cloned().unwrap_or_else(|| json!(0)),
                        "chunk_index": index,
                        "chunk_total": chunk_total,
                        "data": String::from_utf8_lossy(chunk).to_string()
                    }
                });
                let _ = tx.send(envelope);
            }
        }
        normalized.push(json!({
            "artifact_id": artifact_id,
            "type": object.get("type").cloned().unwrap_or_else(|| json!("file")),
            "mime_type": mime_type,
            "filename": object.get("filename").cloned().unwrap_or_else(|| json!("artifact")),
            "bytes": object.get("bytes").cloned().unwrap_or_else(|| json!(encoded_all.len())),
            "width": object.get("width").cloned().unwrap_or_else(|| json!(0)),
            "height": object.get("height").cloned().unwrap_or_else(|| json!(0))
        }));
    }
    Ok(normalized)
}

async fn publish_status(
    writer: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    config: &NodeConfig,
    latency_ms: Option<u64>,
) -> anyhow::Result<()> {
    let status_payload = json!({
        "type": "publish",
        "id": format!("status-{}", uuid::Uuid::now_v7()),
        "topic": "ai.node.device.status",
        "payload": {
            "client_id": config.client_id,
            "runtime_mode": current_runtime_mode(),
            "platform": current_platform_name(),
            "browser_mode": config.browser_mode,
            "browser_preference": config.browser_preference,
            "status": "online",
            "latency_ms": latency_ms
        },
        "meta": {
            "runtime_mode": current_runtime_mode(),
            "platform": current_platform_name(),
            "latency_ms": latency_ms,
            "capabilities": [
                "browser.launch","browser.goto","browser.read","browser.extract","browser.click","browser.type","browser.screenshot",
                "file.list","file.stat","file.read_text","file.open","screen.capture","system.info"
            ]
        }
    });
    writer
        .send(Message::Text(status_payload.to_string()))
        .await
        .context("failed to publish status")?;
    Ok(())
}

async fn handle_ws_text(
    text: &str,
    config: &NodeConfig,
    updates: &Option<mpsc::UnboundedSender<RuntimeUpdate>>,
    last_ping_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    let Ok(payload) = serde_json::from_str::<Value>(text) else {
        warn!(raw = text, "received non-json websocket payload");
        return Ok(());
    };
    let msg_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
    match msg_type {
        "pong" => {
            let latency_ms = last_ping_at.take().map(|item| item.elapsed().as_millis() as u64);
            info!(payload = %payload, latency_ms = ?latency_ms, "received pong");
            if let Some(tx) = updates {
                let _ =
                    tx.send(RuntimeUpdate::Status(status_from_snapshot(config, None, latency_ms)));
            }
        }
        "ack" => info!(payload = %payload, "received ack"),
        "error" => {
            warn!(payload = %payload, "received ws error");
            if let Some(tx) = updates {
                let _ = tx.send(RuntimeUpdate::Error(payload.to_string()));
            }
        }
        "event" => {
            let topic = payload.get("topic").and_then(Value::as_str).unwrap_or("");
            let payload_type = payload
                .get("payload")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if topic == "ai.node.action.request" || payload_type == "ai.node.action.request" {
                println!("[node] received ai.node.action.request {}", payload);
                if let Some(tx) = updates {
                    let request_id = payload
                        .get("meta")
                        .and_then(|meta| meta.get("request_id"))
                        .and_then(Value::as_str)
                        .or_else(|| {
                            payload
                                .get("payload")
                                .and_then(|value| value.get("request_id"))
                                .and_then(Value::as_str)
                        })
                        .unwrap_or_default()
                        .to_string();
                    let event = ActionRequestEvent {
                        request_id,
                        action: payload
                            .get("payload")
                            .and_then(|value| value.get("action"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        payload: payload
                            .get("payload")
                            .and_then(|value| value.get("payload"))
                            .cloned()
                            .unwrap_or_else(|| json!({})),
                    };
                    let _ = tx.send(RuntimeUpdate::ActionRequest(event));
                }
            } else {
                info!(payload = %payload, "received websocket event");
            }
        }
        _ => info!(payload = %payload, "received websocket payload"),
    }
    Ok(())
}

fn status_from_snapshot(
    config: &NodeConfig,
    snapshot: Option<&DeviceRegistration>,
    latency_ms: Option<u64>,
) -> RuntimeStatus {
    RuntimeStatus {
        connected: true,
        latency_ms,
        runtime_mode: current_runtime_mode().to_string(),
        platform: current_platform_name().to_string(),
        client_id: if config.client_id.trim().is_empty() {
            config.device_id.clone()
        } else {
            config.client_id.clone()
        },
        client_name: config.client_name.clone(),
        bound_session: snapshot
            .map(|item| BoundSessionState {
                session_id: item.bound_session_id,
                session_title: item.bound_session_title.clone(),
            })
            .unwrap_or_default(),
    }
}

async fn fetch_device_snapshot(config: &NodeConfig) -> anyhow::Result<DeviceRegistration> {
    let server_url = config.server_url.trim();
    let api_token = config.token.trim();
    if server_url.is_empty() || api_token.is_empty() {
        anyhow::bail!("server_url and token are required before fetching device snapshot");
    }

    let url = format!("{}/agent/v1/devices", server_url.trim_end_matches('/'));
    let client = Client::builder().build().context("failed to build reqwest client")?;
    let response = client
        .get(url)
        .bearer_auth(api_token)
        .send()
        .await
        .context("device list request failed")?;
    let response = response.error_for_status().context("device list response not successful")?;
    let envelope = response
        .json::<ApiEnvelope<Vec<DeviceRegistration>>>()
        .await
        .context("failed to decode device list response")?;

    envelope
        .data
        .into_iter()
        .find(|item| item.device_id.as_deref() == Some(config.device_id.as_str()))
        .ok_or_else(|| anyhow::anyhow!("device snapshot not found for {}", config.device_id))
}

pub fn status_snapshot(config: &NodeConfig) -> RuntimeStatus {
    let client_id = if config.client_id.trim().is_empty() {
        String::new()
    } else {
        config.client_id.clone()
    };
    RuntimeStatus {
        connected: false,
        latency_ms: None,
        runtime_mode: current_runtime_mode().to_string(),
        platform: current_platform_name().to_string(),
        client_id,
        client_name: config.client_name.clone(),
        bound_session: BoundSessionState::default(),
    }
}

pub fn sample_protocol(component: &str, detail: &str) -> ControlMessage {
    ControlMessage::Status(StatusMessage::new(component, "bootstrapped", detail))
}

pub fn current_platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

pub fn current_runtime_mode() -> &'static str {
    if cfg!(target_os = "linux") {
        "daemon"
    } else {
        "tray"
    }
}

async fn connect_ws(
    config: &NodeConfig,
) -> anyhow::Result<(
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Url,
)> {
    let candidates = build_ws_urls(config)?;
    let mut last_error: Option<anyhow::Error> = None;
    for url in candidates {
        match connect_async(url.as_str()).await {
            Ok((socket, _)) => return Ok((socket, url)),
            Err(error) => {
                last_error = Some(
                    anyhow::Error::new(error)
                        .context(format!("failed to connect websocket {}", url)),
                );
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no websocket url candidates available")))
}

fn build_ws_urls(config: &NodeConfig) -> anyhow::Result<Vec<Url>> {
    let base = config.server_url.trim();
    let ws_base = if let Some(stripped) = base.strip_prefix("https://") {
        format!("wss://{}", stripped)
    } else if let Some(stripped) = base.strip_prefix("http://") {
        format!("ws://{}", stripped)
    } else if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_string()
    } else {
        format!("ws://{}", base)
    };
    let mut items: Vec<Url> = Vec::new();

    let mut primary = Url::parse(&format!("{}/ws", ws_base.trim_end_matches('/')))
        .context("invalid primary ws url")?;
    primary
        .query_pairs_mut()
        .append_pair("app", "ai.node")
        .append_pair("client_id", config.client_id.trim())
        .append_pair("device_id", config.device_id.trim())
        .append_pair("token", config.device_id.trim());
    if !items.iter().any(|item| item.as_str() == primary.as_str()) {
        items.push(primary);
    }
    Ok(items)
}
