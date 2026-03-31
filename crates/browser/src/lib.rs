use anyhow::Context;
use base64::Engine;
use dux_ai_node_core::NodeConfig;
use dux_ai_node_platform::ensure_permission;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use sysinfo::{Disks, System};
use tracing::info;
use wait_timeout::ChildExt;
use xcap::image::ImageEncoder;
use xcap::Monitor;

mod backend;
mod platform_helper;

#[derive(Debug, Clone, Serialize)]
pub struct BrowserRuntimeInfo {
    pub runtime_mode: String,
    pub browser_backend: String,
    pub browser_mode: String,
    pub browser_preference: String,
    pub platform_ui_helper: String,
    pub platform_ui_available: bool,
    pub platform_ui_ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActionResponse {
    pub result: Value,
    pub artifacts: Vec<Value>,
    pub meta: Value,
}

pub fn supported_actions() -> &'static [&'static str] {
    &[
        "browser.launch",
        "browser.goto",
        "browser.read",
        "browser.extract",
        "browser.click",
        "browser.type",
        "browser.screenshot",
        "ui.status",
        "app.activate",
        "window.focus",
        "ui.tree",
        "ui.find",
        "ui.read",
        "ui.write",
        "ui.focus",
        "ui.invoke",
        "ui.click",
        "ui.type_native",
        "ui.keypress",
        "file.list",
        "file.stat",
        "file.read_text",
        "file.open",
        "terminal.exec",
        "screen.capture",
        "system.info",
        "channel.qianniu.activate",
        "channel.qianniu.inspect",
        "channel.qianniu.send_text",
        "channel.wechat.current_session",
        "channel.wechat.search_candidates",
        "channel.wechat.open_session",
        "channel.wechat.prepare_text",
        "channel.wechat.send_text",
    ]
}

pub fn runtime_mode() -> &'static str {
    if cfg!(target_os = "linux") {
        "daemon"
    } else {
        "tray"
    }
}

pub fn browser_runtime(config: &NodeConfig) -> BrowserRuntimeInfo {
    let helper = platform_helper::helper_summary();
    BrowserRuntimeInfo {
        runtime_mode: runtime_mode().to_string(),
        browser_backend: backend::browser_backend_name().to_string(),
        browser_mode: config.browser_mode.clone(),
        browser_preference: config.browser_preference.clone(),
        platform_ui_helper: helper
            .get("platform")
            .and_then(Value::as_str)
            .unwrap_or("unsupported")
            .to_string(),
        platform_ui_available: helper.get("available").and_then(Value::as_bool).unwrap_or(false),
        platform_ui_ready: helper.get("ready").and_then(Value::as_bool).unwrap_or(false),
    }
}

fn ensure_browser_permissions() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(status) = platform_helper::execute_helper_action("ax.status", json!({})) {
            let trusted = status.result.get("trusted").and_then(Value::as_bool).unwrap_or(false);
            if trusted {
                return Ok(());
            }
        }
    }
    ensure_permission("accessibility")?;
    Ok(())
}

pub fn execute_action(
    config: &NodeConfig,
    action: &str,
    payload: Value,
) -> anyhow::Result<ActionResponse> {
    info!(action = %action, "executing node action");
    match action {
        a if a.starts_with("browser.") => {
            ensure_browser_permissions()?;
            execute_browser_action(config, a, payload)
        }
        "ui.status" | "app.activate" | "window.focus" | "ui.tree" | "ui.find" | "ui.read"
        | "ui.write" | "ui.focus" | "ui.invoke" | "ui.click" | "ui.type_native" | "ui.keypress"
        | "ax.status" | "ax.tree" => execute_platform_ui_action(action, payload),
        "file.list" => execute_file_list(payload),
        "file.stat" => execute_file_stat(payload),
        "file.read_text" => execute_file_read_text(payload),
        "file.open" => execute_file_open(payload),
        "terminal.exec" => execute_terminal_exec(payload),
        "system.info" => execute_system_info(),
        "screen.capture" => {
            ensure_permission("screen_capture")?;
            execute_screen_capture()
        }
        a if a.starts_with("channel.qianniu.") => execute_qianniu_action(a, payload),
        a if a.starts_with("channel.wechat.") => {
            ensure_browser_permissions()?;
            execute_wechat_action(a, payload)
        }
        _ => anyhow::bail!(format!("unsupported action: {}", action)),
    }
}

fn execute_browser_action(
    config: &NodeConfig,
    action: &str,
    payload: Value,
) -> anyhow::Result<ActionResponse> {
    backend::execute_browser_action(config, action, payload)
}

fn execute_platform_ui_action(action: &str, payload: Value) -> anyhow::Result<ActionResponse> {
    let response = platform_helper::execute_helper_action(action, payload)?;
    Ok(ActionResponse { result: response.result, artifacts: vec![], meta: response.meta })
}

fn execute_qianniu_action(action: &str, payload: Value) -> anyhow::Result<ActionResponse> {
    match action {
        "channel.qianniu.activate" => {
            let request = qianniu_request_payload(&payload, false);
            let response = platform_helper::execute_helper_action("app.activate", request)?;
            Ok(ActionResponse {
                result: json!({
                    "summary": response.result.get("summary").and_then(Value::as_str).unwrap_or("已激活千牛"),
                    "platform": response.result.get("platform").cloned().unwrap_or(Value::Null),
                    "channel": "qianniu",
                    "action": "activate",
                    "details": response.result,
                }),
                artifacts: vec![],
                meta: response.meta,
            })
        }
        "channel.qianniu.inspect" => {
            let request = qianniu_request_payload(&payload, false);
            let response = platform_helper::execute_helper_action("ui.tree", request)?;
            Ok(ActionResponse {
                result: json!({
                    "summary": "已获取千牛窗口信息",
                    "channel": "qianniu",
                    "action": "inspect",
                    "details": response.result,
                }),
                artifacts: vec![],
                meta: response.meta,
            })
        }
        "channel.qianniu.send_text" => {
            let text = payload.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if text.is_empty() {
                anyhow::bail!("text 不能为空")
            }
            let request = qianniu_request_payload(&payload, true);
            let response = platform_helper::execute_helper_action("window.focus", request)?;
            let submit = payload.get("submit").and_then(Value::as_bool).unwrap_or(true);
            backend::type_text(&text, submit)?;
            Ok(ActionResponse {
                result: json!({
                    "summary": if submit { "已向千牛输入并发送消息" } else { "已向千牛输入消息草稿" },
                    "channel": "qianniu",
                    "action": "send_text",
                    "text": text,
                    "submitted": submit,
                    "details": response.result,
                }),
                artifacts: vec![],
                meta: response.meta,
            })
        }
        _ => anyhow::bail!(format!("unsupported action: {}", action)),
    }
}

fn execute_wechat_action(action: &str, payload: Value) -> anyhow::Result<ActionResponse> {
    match action {
        "channel.wechat.current_session" => {
            let session = wechat_current_session()?;
            Ok(ActionResponse {
                result: json!({
                    "summary": format!("当前微信会话：{}", session),
                    "channel": "wechat",
                    "action": "current_session",
                    "session_title": session,
                    "interaction_mode": "safe",
                    "uses_mouse": false,
                    "uses_keyboard": false,
                }),
                artifacts: vec![],
                meta: json!({}),
            })
        }
        "channel.wechat.search_candidates" => {
            let query = required_text(&payload, "query")?;
            wechat_activate()?;
            let data = wechat_search_candidates(&query)?;
            Ok(ActionResponse {
                result: json!({
                    "summary": format!("已获取微信搜索候选：{}", query),
                    "channel": "wechat",
                    "action": "search_candidates",
                    "query": query,
                    "interaction_mode": "active",
                    "uses_mouse": true,
                    "uses_keyboard": false,
                    "candidates": data.get("candidates").cloned().unwrap_or(Value::Null),
                    "selected": data.get("selected").cloned().unwrap_or(Value::Null),
                }),
                artifacts: vec![],
                meta: json!({}),
            })
        }
        "channel.wechat.open_session" => {
            let (query, target_title) = resolve_wechat_target_query(&payload)?;
            wechat_activate()?;
            let detail = wechat_open_session(&query, target_title.as_deref())?;
            let current =
                detail.get("session_title").and_then(Value::as_str).unwrap_or("").to_string();
            Ok(ActionResponse {
                result: json!({
                    "summary": format!("已打开微信会话 {}", current),
                    "channel": "wechat",
                    "action": "open_session",
                    "session_title": current,
                    "interaction_mode": "active",
                    "uses_mouse": true,
                    "uses_keyboard": true,
                    "query": query,
                    "target_title": target_title,
                    "selected": detail.get("selected").cloned().unwrap_or(Value::Null),
                }),
                artifacts: vec![],
                meta: json!({}),
            })
        }
        "channel.wechat.prepare_text" => {
            let session = optional_text(&payload, "session_title");
            let text = required_text(&payload, "text")?;
            if let Some(title) = session.as_deref() {
                wechat_activate()?;
                ensure_wechat_session(title)?;
            }
            let current = wechat_current_session()?;
            wechat_write_text(&text)?;
            Ok(ActionResponse {
                result: json!({
                    "summary": format!("已写入微信输入框（未发送）：{}", current),
                    "channel": "wechat",
                    "action": "prepare_text",
                    "session_title": current,
                    "text": text,
                    "interaction_mode": "safe",
                    "uses_mouse": false,
                    "uses_keyboard": false,
                    "submitted": false,
                }),
                artifacts: vec![],
                meta: json!({}),
            })
        }
        "channel.wechat.send_text" => {
            let mode = optional_text(&payload, "mode").unwrap_or_else(|| "active".to_string());
            let session = optional_text(&payload, "session_title");
            let query = optional_text(&payload, "query");
            let target_title = optional_text(&payload, "target_title");
            let text = required_text(&payload, "text")?;
            wechat_activate()?;
            let open_detail = if let Some(title) = session.as_deref() {
                if mode == "safe" {
                    None
                } else {
                    Some(wechat_open_session(title, None)?)
                }
            } else if let Some(query_value) = query.as_deref() {
                if mode == "safe" {
                    None
                } else {
                    Some(wechat_open_session(query_value, target_title.as_deref())?)
                }
            } else {
                None
            };
            let current = if let Some(title) = session.as_deref() {
                match mode.as_str() {
                    "safe" => ensure_wechat_session(title)?,
                    _ => open_detail
                        .as_ref()
                        .and_then(|detail| detail.get("session_title"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                }
            } else if query.is_some() {
                match mode.as_str() {
                    "safe" => {
                        if let Some(target) = target_title.as_deref() {
                            ensure_wechat_session(target)?
                        } else {
                            wechat_current_session()?
                        }
                    }
                    _ => open_detail
                        .as_ref()
                        .and_then(|detail| detail.get("session_title"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                }
            } else {
                wechat_current_session()?
            };
            wechat_write_text(&text)?;
            if mode == "safe" {
                Ok(ActionResponse {
                    result: json!({
                        "summary": format!("已写入微信输入框草稿：{}", current),
                        "channel": "wechat",
                        "action": "send_text",
                        "mode": "safe",
                        "submitted": false,
                        "session_title": current,
                        "text": text,
                        "uses_mouse": false,
                        "uses_keyboard": false,
                        "query": query,
                        "target_title": target_title,
                    }),
                    artifacts: vec![],
                    meta: json!({}),
                })
            } else {
                wechat_press_return()?;
                Ok(ActionResponse {
                    result: json!({
                        "summary": format!("已向微信会话发送消息：{}", current),
                        "channel": "wechat",
                        "action": "send_text",
                        "mode": "active",
                        "submitted": true,
                        "session_title": current,
                        "text": text,
                        "uses_mouse": false,
                        "uses_keyboard": true,
                        "query": query,
                        "target_title": target_title,
                        "selected": open_detail
                            .as_ref()
                            .and_then(|detail| detail.get("selected"))
                            .cloned()
                            .unwrap_or(Value::Null),
                    }),
                    artifacts: vec![],
                    meta: json!({}),
                })
            }
        }
        _ => anyhow::bail!(format!("unsupported action: {}", action)),
    }
}

fn wechat_activate() -> anyhow::Result<Value> {
    let response = platform_helper::execute_helper_action(
        "app.activate",
        json!({
            "bundle_id": "com.tencent.xinWeChat",
        }),
    )?;
    Ok(response.result)
}

fn wechat_current_session() -> anyhow::Result<String> {
    let input = wechat_primary_text_area()?;
    let title = input.get("title").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if title.is_empty() || title == "输入" || title == "搜索" {
        anyhow::bail!("未能识别当前微信会话")
    }
    Ok(title)
}

fn ensure_wechat_session(expected: &str) -> anyhow::Result<String> {
    let current = wechat_current_session()?;
    if current != expected {
        anyhow::bail!(format!("当前微信会话为 {}，与目标 {} 不一致", current, expected))
    }
    Ok(current)
}

fn wechat_open_session(query: &str, target_title: Option<&str>) -> anyhow::Result<Value> {
    let data = wechat_search_candidates(query)?;
    let candidates =
        data.get("candidates").and_then(Value::as_array).cloned().context("未找到微信候选列表")?;
    let target =
        choose_wechat_candidate(&candidates, target_title)?.context("未找到微信候选结果")?;
    let selected_title = popup_result_title(&target).context("未找到微信结果项标题")?;
    let frame = target.get("frame").cloned().context("未找到微信结果项 frame")?;
    let x = frame.get("center_x").and_then(Value::as_f64).context("微信结果项缺少 center_x")?;
    let y = frame.get("center_y").and_then(Value::as_f64).context("微信结果项缺少 center_y")?;
    platform_helper::execute_helper_action(
        "ui.click",
        json!({
            "x": x,
            "y": y,
        }),
    )?;
    std::thread::sleep(Duration::from_millis(900));

    let current = wechat_current_session()?;
    if current != selected_title {
        anyhow::bail!(format!("微信会话切换失败，当前为 {}", current))
    }
    Ok(json!({
        "query": query,
        "target_title": target_title,
        "session_title": current,
        "selected": target,
        "candidates": data.get("candidates").cloned().unwrap_or(Value::Null),
    }))
}

fn wechat_search_candidates(query: &str) -> anyhow::Result<Value> {
    let search = wechat_search_text_area()?;
    let first_descriptor =
        search.get("descriptor").cloned().context("未找到微信搜索框 descriptor")?;
    platform_helper::execute_helper_action(
        "ui.focus",
        json!({
            "bundle_id": "com.tencent.xinWeChat",
            "element": first_descriptor,
        }),
    )?;
    std::thread::sleep(Duration::from_millis(250));

    let search = wechat_search_text_area()?;
    let second_descriptor =
        search.get("descriptor").cloned().context("未找到微信搜索框 descriptor")?;
    platform_helper::execute_helper_action(
        "ui.write",
        json!({
            "bundle_id": "com.tencent.xinWeChat",
            "element": second_descriptor,
            "text": query,
        }),
    )?;
    std::thread::sleep(Duration::from_millis(250));

    let search = wechat_search_text_area()?;
    let third_descriptor =
        search.get("descriptor").cloned().context("未找到微信搜索框 descriptor")?;
    platform_helper::execute_helper_action(
        "ui.focus",
        json!({
            "bundle_id": "com.tencent.xinWeChat",
            "element": third_descriptor,
        }),
    )?;
    std::thread::sleep(Duration::from_millis(800));

    let candidates = wechat_popup_candidates(query)?;
    let selected = candidates
        .iter()
        .find(|item| item.get("selected").and_then(Value::as_bool) == Some(true))
        .cloned()
        .context("未找到微信候选结果")?;
    Ok(json!({
        "query": query,
        "selected": selected,
        "candidates": candidates,
    }))
}

fn resolve_wechat_target_query(payload: &Value) -> anyhow::Result<(String, Option<String>)> {
    let target_title = optional_text(payload, "target_title");
    let query = if let Some(query) = optional_text(payload, "query") {
        query
    } else {
        required_text(payload, "session_title")?
    };
    Ok((query, target_title))
}

fn wechat_write_text(text: &str) -> anyhow::Result<()> {
    let input = wechat_primary_text_area()?;
    let descriptor = input.get("descriptor").cloned().context("未找到微信输入框 descriptor")?;
    platform_helper::execute_helper_action(
        "ui.write",
        json!({
            "bundle_id": "com.tencent.xinWeChat",
            "element": descriptor,
            "text": text,
        }),
    )?;
    Ok(())
}

fn wechat_press_return() -> anyhow::Result<()> {
    platform_helper::execute_helper_action(
        "ui.keypress",
        json!({
            "bundle_id": "com.tencent.xinWeChat",
            "key": "return",
        }),
    )?;
    Ok(())
}

fn wechat_search_text_area() -> anyhow::Result<Value> {
    let result = platform_helper::execute_helper_action(
        "ui.find",
        json!({
            "bundle_id": "com.tencent.xinWeChat",
            "locator": {
                "role": "AXTextArea",
                "title": "搜索",
                "max_depth": 14
            },
            "limit": 10,
        }),
    )?;
    select_match(&result.result, |item| {
        item.get("role").and_then(Value::as_str) == Some("AXTextArea")
            && item.get("title").and_then(Value::as_str) == Some("搜索")
    })
    .context("未找到微信搜索输入框")
}

fn wechat_popup_candidates(session_title: &str) -> anyhow::Result<Vec<Value>> {
    let tree = platform_helper::execute_helper_action(
        "ui.tree",
        json!({
            "bundle_id": "com.tencent.xinWeChat",
            "max_depth": 2,
        }),
    )?;
    let popup_items = tree
        .result
        .get("windows")
        .and_then(Value::as_array)
        .and_then(|windows| windows.first())
        .and_then(|window| window.get("children"))
        .and_then(Value::as_array)
        .and_then(|children| children.first())
        .and_then(|list| list.get("children"))
        .and_then(Value::as_array)
        .cloned()
        .context("未找到微信搜索弹层结果列表")?;

    let section_labels = ["搜索网络结果", "群聊", "聊天记录", "最近在搜", "最常使用", "最近使用"];
    let mut current_section = String::new();
    let query = session_title.trim();
    let mut candidates: Vec<Value> = vec![];

    for item in popup_items {
        let title = item.get("title").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if title.is_empty() {
            continue;
        }
        if section_labels.contains(&title.as_str()) {
            current_section = title;
            continue;
        }
        if current_section == "搜索网络结果" || current_section == "聊天记录" {
            continue;
        }
        let exact = popup_title_exact(&title, query);
        let contains = popup_title_matches(&title, query);
        let score = popup_match_score(&current_section, exact, contains);
        if score <= 0 {
            continue;
        }
        candidates.push(json!({
            "query": query,
            "session_title": title,
            "section": current_section,
            "match_mode": if exact { "exact" } else { "contains" },
            "score": score,
            "selected": false,
            "frame": item.get("frame").cloned().unwrap_or(Value::Null),
            "window_title": item.get("window_title").cloned().unwrap_or(Value::Null),
        }));
    }

    candidates.sort_by(|a, b| {
        let score_a = a.get("score").and_then(Value::as_i64).unwrap_or_default();
        let score_b = b.get("score").and_then(Value::as_i64).unwrap_or_default();
        score_b.cmp(&score_a)
    });
    if let Some(first) = candidates.first_mut() {
        first["selected"] = Value::Bool(true);
    }
    if candidates.is_empty() {
        anyhow::bail!("未在微信搜索弹层中找到可用结果项")
    }
    Ok(candidates)
}

fn popup_result_title(item: &Value) -> Option<String> {
    let title = item
        .get("session_title")
        .and_then(Value::as_str)
        .or_else(|| item.get("title").and_then(Value::as_str))?
        .trim()
        .to_string();
    (!title.is_empty()).then_some(title)
}

fn popup_title_exact(title: &str, query: &str) -> bool {
    normalize_popup_text(title) == normalize_popup_text(query)
}

fn popup_title_matches(title: &str, query: &str) -> bool {
    let normalized_title = normalize_popup_text(title);
    let tokens = query
        .split_whitespace()
        .map(normalize_popup_text)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return false;
    }
    tokens.iter().all(|token| normalized_title.contains(token))
}

fn normalize_popup_text(value: &str) -> String {
    value.chars().filter(|char| !char.is_whitespace()).collect::<String>().to_lowercase()
}

fn popup_match_score(section: &str, exact: bool, contains: bool) -> i64 {
    let base = match section {
        "群聊" => 400,
        "最近在搜" => 300,
        "最常使用" => 250,
        "最近使用" => 220,
        "" => 180,
        _ => 0,
    };
    if base == 0 {
        return 0;
    }
    if exact {
        return base + 20;
    }
    if contains {
        return base + 10;
    }
    0
}

fn choose_wechat_candidate(
    candidates: &[Value],
    target_title: Option<&str>,
) -> anyhow::Result<Option<Value>> {
    if let Some(target_title) = target_title {
        if let Some(item) = candidates.iter().find(|item| {
            item.get("session_title")
                .and_then(Value::as_str)
                .map(|title| title == target_title)
                .unwrap_or(false)
        }) {
            return Ok(Some(item.clone()));
        }
        if let Some(item) = candidates.iter().find(|item| {
            item.get("session_title")
                .and_then(Value::as_str)
                .map(|title| popup_title_matches(title, target_title))
                .unwrap_or(false)
        }) {
            return Ok(Some(item.clone()));
        }
        anyhow::bail!(format!("未在微信候选中找到目标会话 {}", target_title))
    }
    Ok(candidates
        .iter()
        .find(|item| item.get("selected").and_then(Value::as_bool) == Some(true))
        .cloned())
}

fn wechat_primary_text_area() -> anyhow::Result<Value> {
    let result = platform_helper::execute_helper_action(
        "ui.find",
        json!({
            "bundle_id": "com.tencent.xinWeChat",
            "locator": {
                "role": "AXTextArea",
                "max_depth": 14
            },
            "limit": 20,
        }),
    )?;
    select_match(&result.result, |item| {
        item.get("role").and_then(Value::as_str) == Some("AXTextArea")
            && item.get("title").and_then(Value::as_str) != Some("搜索")
    })
    .context("未找到微信主输入框")
}

fn select_match<F>(result: &Value, predicate: F) -> Option<Value>
where
    F: Fn(&Value) -> bool,
{
    result
        .get("matches")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|item| predicate(item)).cloned())
        .or_else(|| {
            let item = result.get("match")?;
            predicate(item).then(|| item.clone())
        })
}

fn required_text(payload: &Value, key: &str) -> anyhow::Result<String> {
    let value = payload.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
    if value.is_empty() {
        anyhow::bail!(format!("{} 不能为空", key))
    }
    Ok(value)
}

fn optional_text(payload: &Value, key: &str) -> Option<String> {
    let value = payload.get(key).and_then(Value::as_str)?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn qianniu_request_payload(payload: &Value, prefer_window_focus: bool) -> Value {
    let app_name = payload.get("app_name").and_then(Value::as_str).unwrap_or("千牛");
    let process_name = payload.get("process_name").and_then(Value::as_str).unwrap_or("");
    let bundle_id = payload.get("bundle_id").and_then(Value::as_str).unwrap_or("");
    let window_title = payload
        .get("window_title")
        .and_then(Value::as_str)
        .unwrap_or(if prefer_window_focus { "千牛" } else { "" });
    json!({
        "app_name": app_name,
        "process_name": process_name,
        "bundle_id": bundle_id,
        "window_title": window_title,
    })
}

pub fn shutdown_browser_runtime(config: &NodeConfig) {
    backend::shutdown_browser_runtime(config);
}

pub fn cleanup_browser_runtime(config: &NodeConfig) {
    backend::cleanup_browser_runtime(config);
}

fn execute_file_list(payload: Value) -> anyhow::Result<ActionResponse> {
    let path = resolve_path(payload.get("path").and_then(Value::as_str).unwrap_or(""));
    let mut items = Vec::new();
    for entry in
        fs::read_dir(&path).with_context(|| format!("failed to read dir {}", path.display()))?
    {
        let entry = entry?;
        let metadata = entry.metadata()?;
        items.push(json!({
            "name": entry.file_name().to_string_lossy().to_string(),
            "path": entry.path().to_string_lossy().to_string(),
            "is_dir": metadata.is_dir(),
            "size": metadata.len(),
        }));
    }
    Ok(ActionResponse {
        result: json!({ "summary": format!("已列出目录 {}", path.display()), "path": path.to_string_lossy().to_string(), "items": items }),
        artifacts: vec![],
        meta: json!({}),
    })
}

fn execute_file_stat(payload: Value) -> anyhow::Result<ActionResponse> {
    let path = resolve_path(payload.get("path").and_then(Value::as_str).unwrap_or(""));
    let metadata =
        fs::metadata(&path).with_context(|| format!("failed to stat {}", path.display()))?;
    Ok(ActionResponse {
        result: json!({ "summary": format!("已读取文件信息 {}", path.display()), "path": path.to_string_lossy().to_string(), "is_dir": metadata.is_dir(), "size": metadata.len() }),
        artifacts: vec![],
        meta: json!({}),
    })
}

fn execute_file_read_text(payload: Value) -> anyhow::Result<ActionResponse> {
    let path = resolve_path(payload.get("path").and_then(Value::as_str).unwrap_or(""));
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read text {}", path.display()))?;
    Ok(ActionResponse {
        result: json!({ "summary": format!("已读取文本文件 {}", path.display()), "path": path.to_string_lossy().to_string(), "content": content }),
        artifacts: vec![],
        meta: json!({}),
    })
}

fn execute_file_open(payload: Value) -> anyhow::Result<ActionResponse> {
    let path = resolve_path(payload.get("path").and_then(Value::as_str).unwrap_or(""));
    if cfg!(target_os = "linux") {
        anyhow::bail!("file.open is not supported in linux daemon mode")
    }
    let opener = if cfg!(target_os = "macos") { "open" } else { "cmd" };
    let mut command = Command::new(opener);
    if cfg!(target_os = "windows") {
        command.args(["/C", "start", path.to_string_lossy().as_ref()]);
    } else {
        command.arg(&path);
    }
    command.spawn().with_context(|| format!("failed to open {}", path.display()))?;
    Ok(ActionResponse {
        result: json!({ "summary": format!("已打开文件 {}", path.display()), "path": path.to_string_lossy().to_string() }),
        artifacts: vec![],
        meta: json!({}),
    })
}

fn execute_terminal_exec(payload: Value) -> anyhow::Result<ActionResponse> {
    let shell_mode = payload.get("shell").and_then(Value::as_bool).unwrap_or(true);
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(120_000)
        .clamp(1_000, 3_600_000);
    let cwd = resolve_path(payload.get("cwd").and_then(Value::as_str).unwrap_or(""));

    let mut command = if shell_mode {
        let script =
            payload.get("command").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if script.is_empty() {
            anyhow::bail!("command 不能为空")
        }
        shell_command(&script)
    } else {
        let binary =
            payload.get("command").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if binary.is_empty() {
            anyhow::bail!("command 不能为空")
        }
        let mut cmd = Command::new(binary);
        if let Some(args) = payload.get("args").and_then(Value::as_array) {
            for arg in args {
                if let Some(value) = arg.as_str() {
                    cmd.arg(value);
                }
            }
        }
        cmd
    };

    if cwd.as_os_str().is_empty() {
        if let Ok(dir) = std::env::current_dir() {
            command.current_dir(dir);
        }
    } else {
        command.current_dir(&cwd);
    }

    if let Some(envs) = payload.get("env").and_then(Value::as_object) {
        for (key, value) in envs {
            if let Some(text) = value.as_str() {
                command.env(key, text);
            }
        }
    }

    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let started_at = Instant::now();
    let mut child = command.spawn().context("failed to spawn terminal command")?;
    let timeout = Duration::from_millis(timeout_ms);
    let status = match child.wait_timeout(timeout).context("failed to wait terminal command")? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(format!("terminal command timed out after {} ms", timeout_ms))
        }
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }

    let duration_ms = started_at.elapsed().as_millis() as u64;
    let summary =
        if status.success() { "终端命令执行完成" } else { "终端命令执行失败" };

    Ok(ActionResponse {
        result: json!({
            "summary": summary,
            "success": status.success(),
            "exit_code": status.code(),
            "stdout": stdout,
            "stderr": stderr,
            "duration_ms": duration_ms,
            "cwd": command.get_current_dir().map(|item| item.to_string_lossy().to_string()).unwrap_or_default(),
        }),
        artifacts: vec![],
        meta: json!({
            "provider": "terminal",
            "shell": shell_mode,
            "timeout_ms": timeout_ms,
        }),
    })
}

fn shell_command(script: &str) -> Command {
    if cfg!(target_os = "windows") {
        let mut command = Command::new("cmd");
        command.args(["/C", script]);
        return command;
    }

    let shell = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" };
    let mut command = Command::new(shell);
    command.args(["-lc", script]);
    command
}

fn execute_system_info() -> anyhow::Result<ActionResponse> {
    let mut system = System::new_all();
    system.refresh_all();
    let disks = Disks::new_with_refreshed_list();
    let disk_items = disks
        .list()
        .iter()
        .map(|disk| {
            json!({
                "name": disk.name().to_string_lossy().to_string(),
                "mount_point": disk.mount_point().to_string_lossy().to_string(),
                "total_space": disk.total_space(),
                "available_space": disk.available_space(),
            })
        })
        .collect::<Vec<_>>();
    Ok(ActionResponse {
        result: json!({
            "summary": format!("{}，内存 {:.1} GB", System::name().unwrap_or_else(|| std::env::consts::OS.to_string()), system.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0),
            "host_name": System::host_name(),
            "os_name": System::name(),
            "os_version": System::long_os_version().or_else(System::os_version),
            "kernel_version": System::kernel_version(),
            "memory_total": system.total_memory(),
            "memory_used": system.used_memory(),
            "cpus": system.cpus().len(),
            "disks": disk_items,
        }),
        artifacts: vec![],
        meta: json!({}),
    })
}

fn execute_screen_capture() -> anyhow::Result<ActionResponse> {
    let screens = Monitor::all().context("failed to enumerate screens")?;
    let screen = screens.first().context("no screen available")?;
    let image = screen.capture_image().context("failed to capture screen")?;
    let width = image.width();
    let height = image.height();
    let max_side = 1600u32;
    let mut dynamic = xcap::image::DynamicImage::ImageRgba8(image);
    if width > max_side || height > max_side {
        dynamic = dynamic.resize(max_side, max_side, xcap::image::imageops::FilterType::Lanczos3);
    }
    let rgba = dynamic.to_rgba8();
    let out_width = rgba.width();
    let out_height = rgba.height();
    let mut bytes = Vec::new();
    let encoder = xcap::image::codecs::png::PngEncoder::new(&mut bytes);
    encoder
        .write_image(&rgba, out_width, out_height, xcap::image::ColorType::Rgba8.into())
        .context("failed to encode screenshot")?;
    let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    Ok(ActionResponse {
        result: json!({
            "summary": "已截取当前屏幕",
            "mime_type": "image/png",
            "width": out_width,
            "height": out_height,
        }),
        artifacts: vec![json!({
            "type": "image",
            "url": format!("data:image/png;base64,{}", base64),
            "mime_type": "image/png",
            "filename": "screen-capture.png",
            "bytes": bytes.len(),
            "width": out_width,
            "height": out_height,
        })],
        meta: json!({}),
    })
}

fn resolve_path(raw: &str) -> PathBuf {
    let value = raw.trim();
    if value == "~" {
        return std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(stripped) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    PathBuf::from(value)
}
