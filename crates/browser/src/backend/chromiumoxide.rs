use anyhow::{Context, Result};
use base64::Engine;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use dux_ai_node_core::{node_paths, NodeConfig};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::runtime::{Builder as RuntimeBuilder, Handle};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use crate::platform_helper::execute_helper_action;
use crate::ActionResponse;

static SESSION: Lazy<Mutex<Option<ChromiumoxideRuntime>>> = Lazy::new(|| Mutex::new(None));

pub fn browser_backend_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "chromiumoxide-headless"
    } else {
        "chromiumoxide-system-browser"
    }
}

pub fn execute_browser_action(
    config: &NodeConfig,
    action: &str,
    payload: Value,
) -> Result<ActionResponse> {
    let session = with_runtime(|| ensure_runtime(config))?;

    match action {
        "browser.launch" => with_runtime(|| execute_launch(session, config)),
        "browser.goto" => with_runtime(|| execute_goto(session, config, &payload)),
        "browser.read" => with_runtime(|| execute_read(session, config, &payload)),
        "browser.extract" => with_runtime(|| execute_extract(session, config, &payload)),
        "browser.click" => with_runtime(|| execute_click(session, config, &payload)),
        "browser.type" => with_runtime(|| execute_type(session, config, &payload)),
        "browser.screenshot" => with_runtime(|| execute_screenshot(session, config, &payload)),
        _ => anyhow::bail!("chromiumoxide backend does not implement action {} yet", action),
    }
}

pub fn shutdown_browser_runtime(_config: &NodeConfig) {
    let runtime = {
        let mut guard = SESSION.lock().expect("chromiumoxide session lock");
        guard.take()
    };
    if let Some(runtime) = runtime {
        let _ = with_runtime(|| async move {
            runtime.shutdown().await;
            Ok(())
        });
    }
}

pub fn cleanup_browser_runtime(_config: &NodeConfig) {}

struct ChromiumoxideRuntime {
    fingerprint: String,
    browser: Browser,
    page: Page,
    handler_task: JoinHandle<()>,
}

impl ChromiumoxideRuntime {
    async fn shutdown(mut self) {
        let _ = self.page.close().await;
        let _ = self.browser.close().await;
        let _ = self.browser.wait().await;
        self.handler_task.abort();
    }
}

async fn ensure_runtime(config: &NodeConfig) -> Result<(Page, String)> {
    let fingerprint = runtime_fingerprint(config);

    let stale = {
        let guard = SESSION.lock().expect("chromiumoxide session lock");
        guard.as_ref().map(|item| item.fingerprint != fingerprint).unwrap_or(false)
    };
    if stale {
        shutdown_browser_runtime(config);
    }

    {
        let guard = SESSION.lock().expect("chromiumoxide session lock");
        if let Some(runtime) = guard.as_ref() {
            return Ok((runtime.page.clone(), runtime.fingerprint.clone()));
        }
    }

    let launched = launch_runtime(config).await?;
    let page = launched.page.clone();
    let fingerprint = launched.fingerprint.clone();

    let mut guard = SESSION.lock().expect("chromiumoxide session lock");
    *guard = Some(launched);
    Ok((page, fingerprint))
}

async fn launch_runtime(config: &NodeConfig) -> Result<ChromiumoxideRuntime> {
    let browser_config = build_browser_config(config)?;
    let (browser, mut handler) =
        Browser::launch(browser_config).await.context("failed to launch chromiumoxide browser")?;

    let handler_task = tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if event.is_err() {
                break;
            }
        }
    });

    let page =
        browser.new_page("about:blank").await.context("failed to create chromiumoxide page")?;

    if !is_headless(config) {
        focus_browser_window(config, None);
        let _ = page.bring_to_front().await;
        let _ = page.activate().await;
    }

    Ok(ChromiumoxideRuntime {
        fingerprint: runtime_fingerprint(config),
        browser,
        page,
        handler_task,
    })
}

async fn execute_launch((page, _): (Page, String), config: &NodeConfig) -> Result<ActionResponse> {
    if !is_headless(config) {
        focus_browser_window(config, None);
        let _ = page.bring_to_front().await;
        let _ = page.activate().await;
    }
    let url = page.url().await?.unwrap_or_else(|| "about:blank".to_string());
    let title = page_title(&page).await.unwrap_or_default();
    Ok(ActionResponse {
        result: json!({
            "summary": "已启动自动化浏览器",
            "url": url,
            "title": title,
            "mode": normalized_mode(config),
            "browser": config.browser_preference,
        }),
        artifacts: vec![],
        meta: json!({
            "mode": normalized_mode(config),
            "browser": config.browser_preference,
        }),
    })
}

async fn execute_goto(
    (page, _): (Page, String),
    config: &NodeConfig,
    payload: &Value,
) -> Result<ActionResponse> {
    let url = payload.get("url").and_then(Value::as_str).unwrap_or("").trim();
    if url.is_empty() {
        anyhow::bail!("url 不能为空")
    }
    navigate_with_timeout(&page, payload, url).await?;
    let current_url = page.url().await?.unwrap_or_else(|| url.to_string());
    let title = page_title(&page).await.unwrap_or_default();
    if !is_headless(config) {
        focus_browser_window(config, Some(title.as_str()));
        let _ = page.bring_to_front().await;
        let _ = page.activate().await;
    }
    Ok(ActionResponse {
        result: json!({
            "summary": format!("已在自动化浏览器打开 {}", current_url),
            "url": current_url,
            "title": title,
            "mode": normalized_mode(config),
            "browser": config.browser_preference,
        }),
        artifacts: vec![],
        meta: json!({
            "mode": normalized_mode(config),
            "browser": config.browser_preference,
        }),
    })
}

async fn execute_read(
    (page, _): (Page, String),
    config: &NodeConfig,
    payload: &Value,
) -> Result<ActionResponse> {
    navigate_if_needed(&page, payload).await?;
    if !is_headless(config) {
        let title = page_title(&page).await.unwrap_or_default();
        focus_browser_window(config, Some(title.as_str()));
        let _ = page.bring_to_front().await;
    }
    build_extract_response(page, config, payload, true).await
}

async fn execute_extract(
    (page, _): (Page, String),
    config: &NodeConfig,
    payload: &Value,
) -> Result<ActionResponse> {
    navigate_if_needed(&page, payload).await?;
    build_extract_response(page, config, payload, false).await
}

async fn execute_click(
    (page, _): (Page, String),
    config: &NodeConfig,
    payload: &Value,
) -> Result<ActionResponse> {
    let selector = payload.get("selector").and_then(Value::as_str).unwrap_or("").trim();
    if selector.is_empty() {
        anyhow::bail!("selector 不能为空")
    }

    let element = page
        .find_element(selector)
        .await
        .with_context(|| format!("selector 不存在或不可用: {}", selector))?;
    element.click().await.context("failed to click element")?;
    wait_for_navigation_if_requested(&page, payload).await;

    let current_url = page.url().await?.unwrap_or_default();
    let title = page_title(&page).await.unwrap_or_default();
    if !is_headless(config) {
        focus_browser_window(config, Some(title.as_str()));
    }
    Ok(ActionResponse {
        result: json!({
            "summary": format!("已点击元素 {}", selector),
            "url": current_url,
            "title": title,
            "selector": selector,
            "mode": normalized_mode(config),
        }),
        artifacts: vec![],
        meta: json!({
            "mode": normalized_mode(config),
            "browser": config.browser_preference,
        }),
    })
}

async fn execute_type(
    (page, _): (Page, String),
    config: &NodeConfig,
    payload: &Value,
) -> Result<ActionResponse> {
    let selector = payload.get("selector").and_then(Value::as_str).unwrap_or("").trim();
    if selector.is_empty() {
        anyhow::bail!("selector 不能为空")
    }
    let text = payload.get("text").and_then(Value::as_str).unwrap_or("");
    let clear = payload.get("clear").and_then(Value::as_bool).unwrap_or(true);
    let press_enter = payload.get("press_enter").and_then(Value::as_bool).unwrap_or(false);

    let element = page
        .find_element(selector)
        .await
        .with_context(|| format!("selector 不存在或不可用: {}", selector))?;

    element.focus().await.context("failed to focus element")?;
    if clear {
        element
            .call_js_fn("function() { if ('value' in this) { this.value = ''; } }", true)
            .await
            .context("failed to clear element value")?;
    }
    if !text.is_empty() {
        element.type_str(text).await.context("failed to type text")?;
    }
    if press_enter {
        element.press_key("Enter").await.context("failed to press enter")?;
        wait_for_navigation_if_requested(&page, payload).await;
    }

    let current_url = page.url().await?.unwrap_or_default();
    let title = page_title(&page).await.unwrap_or_default();
    if !is_headless(config) {
        focus_browser_window(config, Some(title.as_str()));
    }
    Ok(ActionResponse {
        result: json!({
            "summary": format!("已输入内容到元素 {}", selector),
            "url": current_url,
            "title": title,
            "selector": selector,
            "text": text,
            "mode": normalized_mode(config),
        }),
        artifacts: vec![],
        meta: json!({
            "mode": normalized_mode(config),
            "browser": config.browser_preference,
        }),
    })
}

async fn execute_screenshot(
    (page, _): (Page, String),
    config: &NodeConfig,
    _payload: &Value,
) -> Result<ActionResponse> {
    let params = ScreenshotParams::builder().format(CaptureScreenshotFormat::Png).build();
    let bytes = page.screenshot(params).await.context("failed to capture page screenshot")?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let url = page.url().await?.unwrap_or_default();
    Ok(ActionResponse {
        result: json!({
            "summary": "已截取当前浏览器页面",
            "url": url,
            "mime_type": "image/png",
            "mode": normalized_mode(config),
        }),
        artifacts: vec![json!({
            "type": "image",
            "url": format!("data:image/png;base64,{}", encoded),
            "mime_type": "image/png",
            "filename": "browser-screenshot.png",
            "bytes": bytes.len(),
        })],
        meta: json!({
            "mode": normalized_mode(config),
            "browser": config.browser_preference,
        }),
    })
}

async fn build_extract_response(
    page: Page,
    config: &NodeConfig,
    payload: &Value,
    read_mode: bool,
) -> Result<ActionResponse> {
    let current_url = page.url().await?.unwrap_or_default();
    let title = page_title(&page).await.unwrap_or_default();
    let mode = payload.get("mode").and_then(Value::as_str).unwrap_or("text").trim();
    let selector = payload.get("selector").and_then(Value::as_str).unwrap_or("").trim();

    let result = if !selector.is_empty() {
        let element = page
            .find_element(selector)
            .await
            .with_context(|| format!("selector 不存在或不可用: {}", selector))?;
        if mode == "html" {
            let html = element.outer_html().await?.unwrap_or_default();
            json!({
                "url": current_url,
                "title": title,
                "selector": selector,
                "html": html,
                "summary": if read_mode { format!("已打开并读取网页 {}", current_url) } else { format!("已提取 {} 的 HTML", selector) },
                "mode": normalized_mode(config),
            })
        } else {
            let text = element.inner_text().await?.unwrap_or_default();
            json!({
                "url": current_url,
                "title": title,
                "selector": selector,
                "items": [text],
                "summary": if read_mode { format!("已打开并读取网页 {}", current_url) } else { format!("已提取 {} 的节点内容", selector) },
                "mode": normalized_mode(config),
            })
        }
    } else if mode == "html" {
        let html = page.content().await?;
        json!({
            "url": current_url,
            "title": title,
            "html": html,
            "summary": if read_mode { format!("已打开并读取网页 {}", current_url) } else { format!("已提取网页 {} 的 HTML", current_url) },
            "mode": normalized_mode(config),
        })
    } else {
        let content: String = page
            .evaluate("document.body ? document.body.innerText : ''")
            .await?
            .into_value()
            .unwrap_or_default();
        json!({
            "url": current_url,
            "title": title,
            "content": content.trim(),
            "summary": if read_mode { format!("已打开并读取网页 {}", current_url) } else { format!("已提取网页 {}", current_url) },
            "mode": normalized_mode(config),
        })
    };

    Ok(ActionResponse {
        result,
        artifacts: vec![],
        meta: json!({
            "mode": normalized_mode(config),
            "browser": config.browser_preference,
        }),
    })
}

async fn navigate_if_needed(page: &Page, payload: &Value) -> Result<()> {
    let url = payload.get("url").and_then(Value::as_str).unwrap_or("").trim();
    if !url.is_empty() {
        navigate_with_timeout(page, payload, url).await?;
    }
    Ok(())
}

async fn wait_for_navigation_if_requested(page: &Page, payload: &Value) {
    let wait_requested = payload
        .get("wait_until")
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if wait_requested {
        let _ = timeout(navigation_timeout(payload), page.wait_for_navigation()).await;
    }
}

async fn navigate_with_timeout(page: &Page, payload: &Value, url: &str) -> Result<()> {
    timeout(navigation_timeout(payload), page.goto(url))
        .await
        .with_context(|| format!("page navigation timeout: {}", url))?
        .with_context(|| format!("failed to navigate page: {}", url))?;
    Ok(())
}

fn navigation_timeout(payload: &Value) -> Duration {
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .or_else(|| payload.get("navigation_timeout_ms").and_then(Value::as_u64))
        .unwrap_or(15_000);
    Duration::from_millis(timeout_ms.clamp(1_000, 120_000))
}

fn focus_browser_window(config: &NodeConfig, window_title: Option<&str>) {
    let payload = browser_helper_payload(config, window_title);
    let _ = execute_helper_action("app.activate", payload.clone());
    let _ = execute_helper_action("window.focus", payload);
}

fn browser_helper_payload(config: &NodeConfig, window_title: Option<&str>) -> Value {
    let app_name = match config.browser_preference.trim().to_ascii_lowercase().as_str() {
        "msedge" | "edge" => "Microsoft Edge",
        _ => "Google Chrome",
    };
    let bundle_id = match app_name {
        "Microsoft Edge" => "com.microsoft.edgemac",
        _ => "com.google.Chrome",
    };
    json!({
        "bundle_id": bundle_id,
        "app_name": app_name,
        "window_title": window_title.unwrap_or(""),
    })
}

async fn page_title(page: &Page) -> Result<String> {
    Ok(page.evaluate("document.title || ''").await?.into_value().unwrap_or_default())
}

fn build_browser_config(config: &NodeConfig) -> Result<BrowserConfig> {
    let profile_dir = browser_profile_dir(config);
    let mut builder = BrowserConfig::builder().user_data_dir(profile_dir).viewport(None);

    if is_headless(config) {
        builder = builder.no_sandbox();
    } else {
        builder = builder.with_head().window_size(1440, 900);
    }

    if let Some(executable) = resolve_browser_executable(config) {
        builder = builder.chrome_executable(executable);
    }

    builder.build().map_err(anyhow::Error::msg)
}

fn resolve_browser_executable(config: &NodeConfig) -> Option<PathBuf> {
    match config.browser_preference.trim().to_ascii_lowercase().as_str() {
        "chrome" => detect_chrome_path(),
        "msedge" | "edge" => detect_edge_path(),
        _ => detect_chrome_path().or_else(detect_edge_path),
    }
}

fn detect_chrome_path() -> Option<PathBuf> {
    detect_existing_path(&[
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "C:/Program Files/Google/Chrome/Application/chrome.exe",
        "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
    ])
}

fn detect_edge_path() -> Option<PathBuf> {
    detect_existing_path(&[
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
        "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
        "/usr/bin/microsoft-edge",
        "/usr/bin/microsoft-edge-stable",
    ])
}

fn detect_existing_path(candidates: &[&str]) -> Option<PathBuf> {
    candidates.iter().map(PathBuf::from).find(|path| path.exists())
}

fn runtime_fingerprint(config: &NodeConfig) -> String {
    format!("{}:{}:{}", config.device_id, config.browser_preference, normalized_mode(config))
}

fn normalized_mode(config: &NodeConfig) -> &'static str {
    if is_headless(config) {
        "headless"
    } else {
        "headed"
    }
}

fn is_headless(config: &NodeConfig) -> bool {
    config.browser_mode.trim().eq_ignore_ascii_case("headless")
}

fn browser_profile_dir(config: &NodeConfig) -> PathBuf {
    if let Ok(paths) = node_paths() {
        return paths.data_dir.join("browser-profile").join(&config.device_id);
    }
    std::env::temp_dir().join("dux-ai-node-browser-profile").join(&config.device_id)
}

fn with_runtime<F, Fut, T>(factory: F) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    if let Ok(handle) = Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(factory()))
    } else {
        let runtime = RuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to create ad-hoc runtime")?;
        runtime.block_on(factory())
    }
}
