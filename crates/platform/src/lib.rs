use anyhow::Context;
use dux_ai_node_core::{node_paths, NodePaths};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub fn paths() -> anyhow::Result<NodePaths> {
    node_paths()
}

pub fn current_platform() -> &'static str {
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

pub fn daemon_entry() -> &'static str {
    "apps/node-daemon"
}

pub fn tray_entry() -> &'static str {
    "apps/node-tray"
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionStatus {
    pub id: String,
    pub status: String,
    pub summary: String,
    pub detail: String,
}

#[cfg(target_os = "macos")]
pub fn permission_statuses() -> Vec<PermissionStatus> {
    let full_disk_access = full_disk_access_status();
    vec![
        PermissionStatus {
            id: "app_management".to_string(),
            status: "entry".to_string(),
            summary: "App 管理检查".to_string(),
            detail: "请检查“通用 -> App 管理”中 Dux AI Node 是否存在且开关已开启。".to_string(),
        },
        PermissionStatus {
            id: "screen_capture".to_string(),
            status: if screen_capture_granted() { "granted" } else { "denied" }.to_string(),
            summary: if screen_capture_granted() {
                "屏幕录制已授权"
            } else {
                "屏幕录制未授权"
            }
            .to_string(),
            detail: "截图与视觉类动作依赖该权限。".to_string(),
        },
        PermissionStatus {
            id: "accessibility".to_string(),
            status: if accessibility_granted() { "granted" } else { "denied" }.to_string(),
            summary: if accessibility_granted() {
                "辅助功能已授权"
            } else {
                "辅助功能未授权"
            }
            .to_string(),
            detail: "浏览器点击、输入、窗口控制通常需要辅助功能权限。".to_string(),
        },
        PermissionStatus {
            id: "full_disk_access".to_string(),
            status: full_disk_access.status,
            summary: full_disk_access.summary,
            detail: "文件读取/遍历能力已支持；访问 Mail、Safari、Messages 等受保护目录时通常还需要完全磁盘访问权限。".to_string(),
        },
        PermissionStatus {
            id: "automation".to_string(),
            status: "unknown".to_string(),
            summary: "自动化权限按需触发".to_string(),
            detail: "首次控制浏览器时，macOS 可能弹出自动化授权。".to_string(),
        },
        PermissionStatus {
            id: "terminal_control".to_string(),
            status: "enabled".to_string(),
            summary: "终端控制已启用".to_string(),
            detail: "节点已支持执行终端命令；高风险命令应使用确认模式，并建议先调用 system.info 判断系统再执行。".to_string(),
        },
    ]
}

#[cfg(not(target_os = "macos"))]
pub fn permission_statuses() -> Vec<PermissionStatus> {
    vec![]
}

#[cfg(target_os = "macos")]
pub fn open_permission_settings(permission: &str) -> anyhow::Result<()> {
    let url = match permission {
        "app_management" => "x-apple.systempreferences:com.apple.systempreferences.GeneralSettings",
        "screen_capture" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        "accessibility" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        "full_disk_access" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
        }
        "automation" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation"
        }
        _ => anyhow::bail!("unsupported permission: {}", permission),
    };
    info!(permission = %permission, url = %url, "opening macOS permission settings");
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .context("failed to open macOS privacy settings")?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn open_permission_settings(_permission: &str) -> anyhow::Result<()> {
    anyhow::bail!("permission settings deeplink is only implemented for macOS")
}

#[cfg(target_os = "macos")]
pub fn open_privacy_settings() -> anyhow::Result<()> {
    let url = "x-apple.systempreferences:com.apple.preference.security";
    info!(url = %url, "opening macOS privacy settings root");
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .context("failed to open macOS privacy settings root")?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn open_privacy_settings() -> anyhow::Result<()> {
    anyhow::bail!("privacy settings deeplink is only implemented for macOS")
}

#[cfg(target_os = "macos")]
fn screen_capture_granted() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> i32;
    }
    unsafe { CGPreflightScreenCaptureAccess() != 0 }
}

#[cfg(target_os = "macos")]
fn accessibility_granted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    unsafe { AXIsProcessTrusted() }
}

#[cfg(target_os = "macos")]
fn full_disk_access_status() -> PermissionStatus {
    let home = match std::env::var("HOME") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => {
            return PermissionStatus {
                id: "full_disk_access".to_string(),
                status: "unknown".to_string(),
                summary: "完全磁盘访问待检查".to_string(),
                detail: "未能定位当前用户目录，无法检测完全磁盘访问权限。".to_string(),
            };
        }
    };

    let targets = [
        home.join("Library/Mail"),
        home.join("Library/Safari"),
        home.join("Library/Messages"),
    ];

    let mut found_protected_target = false;
    let mut permission_denied = false;

    for path in targets {
        if !path.exists() {
            continue;
        }
        found_protected_target = true;

        match fs::read_dir(&path) {
            Ok(_) => {
                return PermissionStatus {
                    id: "full_disk_access".to_string(),
                    status: "granted".to_string(),
                    summary: "完全磁盘访问可用".to_string(),
                    detail: format!("已可访问受保护目录：{}", path.display()),
                };
            }
            Err(error) => {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    permission_denied = true;
                }
            }
        }
    }

    if permission_denied {
        return PermissionStatus {
            id: "full_disk_access".to_string(),
            status: "denied".to_string(),
            summary: "完全磁盘访问未授权".to_string(),
            detail: "检测到受保护目录不可访问，访问 Mail、Safari、Messages 等目录时会受限。".to_string(),
        };
    }

    if found_protected_target {
        return PermissionStatus {
            id: "full_disk_access".to_string(),
            status: "unknown".to_string(),
            summary: "完全磁盘访问待确认".to_string(),
            detail: "目录存在但当前无法稳定判断授权状态，可在系统设置中手动确认。".to_string(),
        };
    }

    PermissionStatus {
        id: "full_disk_access".to_string(),
        status: "unknown".to_string(),
        summary: "完全磁盘访问待确认".to_string(),
        detail: "当前机器未发现可用于探测的受保护目录，可按需在系统设置中开启。".to_string(),
    }
}

#[cfg(target_os = "macos")]
pub fn ensure_permission(permission: &str) -> anyhow::Result<()> {
    let statuses = permission_statuses();
    if let Some(item) = statuses.iter().find(|item| item.id == permission) {
        if item.status == "granted" || item.status == "unknown" {
            info!(permission = %permission, status = %item.status, "permission check passed");
            return Ok(());
        }
        warn!(permission = %permission, status = %item.status, "permission check failed");
        anyhow::bail!(format!("permission_required:{}:{}", item.id, item.summary))
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn ensure_permission(_permission: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn launch_agent_path() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/LaunchAgents/plus.dux.ai.node.plist"))
}

#[cfg(target_os = "macos")]
pub fn install_launch_agent(executable_path: &Path) -> anyhow::Result<PathBuf> {
    let path = launch_agent_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>plus.dux.ai.node</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>run</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ProcessType</key>
  <string>Interactive</string>
</dict>
</plist>
"#,
        executable_path.display()
    );
    fs::write(&path, plist).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

#[cfg(target_os = "macos")]
pub fn uninstall_launch_agent() -> anyhow::Result<()> {
    let path = launch_agent_path()?;
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn launch_agent_installed() -> anyhow::Result<bool> {
    Ok(launch_agent_path()?.exists())
}

#[cfg(not(target_os = "macos"))]
pub fn install_launch_agent(_executable_path: &Path) -> anyhow::Result<PathBuf> {
    anyhow::bail!("launch agent install is only implemented for macOS")
}

#[cfg(not(target_os = "macos"))]
pub fn uninstall_launch_agent() -> anyhow::Result<()> {
    anyhow::bail!("launch agent uninstall is only implemented for macOS")
}

#[cfg(not(target_os = "macos"))]
pub fn launch_agent_installed() -> anyhow::Result<bool> {
    Ok(false)
}

pub fn clear_log_files() -> anyhow::Result<()> {
    let paths = node_paths()?;
    std::fs::create_dir_all(&paths.log_dir)?;
    std::fs::write(paths.log_dir.join("node.log"), "")?;
    std::fs::write(paths.log_dir.join("connection.log"), "")?;
    Ok(())
}

pub fn current_executable_path() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

pub fn current_application_path() -> String {
    let exe = std::env::current_exe().ok();
    if let Some(exe) = exe {
        let rendered = exe.display().to_string();
        if let Some(index) = rendered.find(".app/") {
            return format!("{}.app", &rendered[..index]);
        }
        return rendered;
    }
    "unknown".to_string()
}

#[cfg(target_os = "macos")]
pub fn relaunch_current_application() -> anyhow::Result<()> {
    let app_path = current_application_path();
    std::process::Command::new("open")
        .args(["-n", &app_path])
        .spawn()
        .context("failed to relaunch macOS app")?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn relaunch_current_application() -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    std::process::Command::new(exe).spawn().context("failed to relaunch application")?;
    Ok(())
}
