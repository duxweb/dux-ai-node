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
        "file.list",
        "file.stat",
        "file.read_text",
        "file.open",
        "terminal.exec",
        "screen.capture",
        "system.info",
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
        platform_ui_helper: helper.get("platform").and_then(Value::as_str).unwrap_or("unsupported").to_string(),
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
        let script = payload
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if script.is_empty() {
            anyhow::bail!("command 不能为空")
        }
        shell_command(&script)
    } else {
        let binary = payload
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
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
    let summary = if status.success() {
        "终端命令执行完成"
    } else {
        "终端命令执行失败"
    };

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
