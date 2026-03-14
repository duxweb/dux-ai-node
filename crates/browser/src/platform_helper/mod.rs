#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;

// This module only abstracts platform semantic UI control.
// Shared browser automation stays in chromiumoxide, and low-level fallback input stays in enigo.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformUiRequest {
    pub id: Option<String>,
    pub action: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlatformUiResponse {
    pub id: Option<String>,
    pub ok: bool,
    pub result: Value,
    pub meta: Value,
    pub error: Option<String>,
}

pub trait PlatformUiHelper: Send + Sync {
    fn platform(&self) -> &'static str;

    fn available(&self) -> bool;

    fn execute(&self, request: &PlatformUiRequest) -> anyhow::Result<PlatformUiResponse>;
}

pub fn current_helper() -> Box<dyn PlatformUiHelper> {
    #[cfg(target_os = "macos")]
    {
        return Box::new(macos::MacosUiHelper::default());
    }

    #[cfg(target_os = "windows")]
    {
        return Box::new(windows::WindowsUiHelper::default());
    }

    #[cfg(target_os = "linux")]
    {
        return Box::new(linux::LinuxUiHelper::default());
    }

    #[allow(unreachable_code)]
    Box::new(unsupported::UnsupportedUiHelper::default())
}

pub fn helper_summary() -> Value {
    let helper = current_helper();
    serde_json::json!({
        "platform": helper.platform(),
        "available": helper.available(),
    })
}

pub fn execute_helper_action(action: &str, payload: Value) -> anyhow::Result<PlatformUiResponse> {
    let helper = current_helper();
    helper.execute(&PlatformUiRequest {
        id: Some(format!("platform-ui-{}", action.replace('.', "-"))),
        action: action.to_string(),
        payload,
    })
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{PlatformUiHelper, PlatformUiRequest, PlatformUiResponse};
    use super::{helper_resource_root, resolve_swift_binary};
    use anyhow::Context;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::Stdio;

    #[derive(Default)]
    pub struct MacosUiHelper;

    impl PlatformUiHelper for MacosUiHelper {
        fn platform(&self) -> &'static str {
            "macos"
        }

        fn available(&self) -> bool {
            helper_executable_path().is_some() || helper_package_path().is_some()
        }

        fn execute(&self, request: &PlatformUiRequest) -> anyhow::Result<PlatformUiResponse> {
            let raw = invoke_helper(request).with_context(|| {
                format!("failed to execute macOS UI helper action {}", request.action)
            })?;
            if raw.ok {
                Ok(raw)
            } else {
                anyhow::bail!(raw.error.unwrap_or_else(|| format!("macOS UI helper action failed: {}", request.action)))
            }
        }
    }

    fn invoke_helper(request: &PlatformUiRequest) -> anyhow::Result<PlatformUiResponse> {
        let command = helper_command()?;
        let output = if command.0 == "swift" {
            let mut child = std::process::Command::new(&command.0)
                .args(&command.1)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context("failed to spawn swift helper")?;

            if let Some(stdin) = child.stdin.as_mut() {
                let payload = serde_json::to_string(request)?;
                stdin.write_all(payload.as_bytes())?;
                stdin.write_all(b"\n")?;
            }
            child.wait_with_output().context("failed to wait for swift helper output")?
        } else {
            let mut child = std::process::Command::new(&command.0)
                .args(&command.1)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context("failed to spawn macOS UI helper binary")?;

            if let Some(stdin) = child.stdin.as_mut() {
                let payload = serde_json::to_string(request)?;
                stdin.write_all(payload.as_bytes())?;
                stdin.write_all(b"\n")?;
            }
            child.wait_with_output().context("failed to wait for macOS UI helper output")?
        };

        if !output.status.success() && output.stdout.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!(if stderr.is_empty() { "macOS UI helper exited unexpectedly".to_string() } else { stderr })
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.lines().find(|item| !item.trim().is_empty()).context("macOS UI helper returned empty response")?;
        let response: PlatformUiResponse = serde_json::from_str(line).context("failed to decode macOS UI helper response")?;
        Ok(response)
    }

    fn helper_command() -> anyhow::Result<(String, Vec<String>)> {
        if let Some(binary) = helper_executable_path() {
            return Ok((binary.to_string_lossy().to_string(), vec![]));
        }
        let package = helper_package_path().context("failed to locate macOS helper package")?;
        let swift = resolve_swift_binary().context("swift toolchain is unavailable")?;
        Ok((
            swift,
            vec![
                "run".to_string(),
                "--package-path".to_string(),
                package.to_string_lossy().to_string(),
                "dux-node-macos-ax-helper".to_string(),
            ],
        ))
    }

    fn helper_package_path() -> Option<PathBuf> {
        let root = helper_resource_root()?;
        let path = root.join("helpers/macos-ax-helper");
        path.join("Package.swift").exists().then_some(path)
    }

    fn helper_executable_path() -> Option<PathBuf> {
        let root = helper_resource_root()?;
        let candidates = [
            root.join("helpers/macos-ax-helper/.build/release/dux-node-macos-ax-helper"),
            root.join("helpers/macos-ax-helper/.build/debug/dux-node-macos-ax-helper"),
            root.join("runtime/helpers/dux-node-macos-ax-helper"),
        ];
        candidates.into_iter().find(|path| path.exists())
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::{PlatformUiHelper, PlatformUiRequest, PlatformUiResponse};

    #[derive(Default)]
    pub struct WindowsUiHelper;

    impl PlatformUiHelper for WindowsUiHelper {
        fn platform(&self) -> &'static str {
            "windows"
        }

        fn available(&self) -> bool {
            false
        }

        fn execute(&self, request: &PlatformUiRequest) -> anyhow::Result<PlatformUiResponse> {
            anyhow::bail!("Windows UI helper is planned but not implemented yet for action {}", request.action)
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{PlatformUiHelper, PlatformUiRequest, PlatformUiResponse};

    #[derive(Default)]
    pub struct LinuxUiHelper;

    impl PlatformUiHelper for LinuxUiHelper {
        fn platform(&self) -> &'static str {
            "linux"
        }

        fn available(&self) -> bool {
            false
        }

        fn execute(&self, request: &PlatformUiRequest) -> anyhow::Result<PlatformUiResponse> {
            anyhow::bail!("Linux runtime does not expose UI automation helper for action {}", request.action)
        }
    }
}

mod unsupported {
    use super::{PlatformUiHelper, PlatformUiRequest, PlatformUiResponse};

    #[derive(Default)]
    pub struct UnsupportedUiHelper;

    impl PlatformUiHelper for UnsupportedUiHelper {
        fn platform(&self) -> &'static str {
            "unsupported"
        }

        fn available(&self) -> bool {
            false
        }

        fn execute(&self, request: &PlatformUiRequest) -> anyhow::Result<PlatformUiResponse> {
            anyhow::bail!("Unsupported platform UI helper for action {}", request.action)
        }
    }
}

fn helper_resource_root() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("DUX_AI_NODE_ROOT") {
        let path = PathBuf::from(root);
        if path.join("helpers").exists() {
            return Some(path);
        }
    }

    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.to_path_buf());
            candidates.push(dir.join("../Resources"));
            candidates.push(dir.join("resources"));
            candidates.push(dir.join("../resources"));
        }
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));

    candidates.into_iter().find(|path| path.join("helpers").exists())
}

fn resolve_swift_binary() -> Option<String> {
    let candidates = ["swift", "/usr/bin/swift"];
    candidates
        .iter()
        .find(|candidate| std::process::Command::new(candidate).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|status| status.success()).unwrap_or(false))
        .map(|candidate| (*candidate).to_string())
}
