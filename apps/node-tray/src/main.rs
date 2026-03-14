#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dux_ai_node_browser::{browser_runtime, execute_action, shutdown_browser_runtime};
use dux_ai_node_core::{
    logging::{init_logging, resolve_log_files},
    node_paths,
    runtime::{
        ensure_registration, publish_action_result, run_runtime_with_updates, status_snapshot,
        RuntimeUpdate,
    },
    BoundSessionState, NodeConfig, RuntimeStatus,
};
use image::ImageFormat;
use minijinja::{context, Environment};
#[cfg(target_os = "windows")]
use std::fs;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use tokio::task::JoinHandle;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

#[cfg(target_os = "macos")]
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS, EventLoopWindowTargetExtMacOS};
use tray_icon::{Icon, TrayIconBuilder};
use url::Url;
use wry::WebViewBuilder;
#[cfg(target_os = "windows")]
use wry::WebContext;

use dux_ai_node_platform::{
    clear_log_files, current_application_path, install_launch_agent, launch_agent_installed,
    open_permission_settings, open_privacy_settings, permission_statuses,
    relaunch_current_application, uninstall_launch_agent,
};

#[derive(Parser, Debug)]
#[command(name = "dux-ai-node", about = "Dux AI node tray")]
struct Cli {
    #[arg(long)]
    config: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Register,
    Status,
    Autostart {
        #[command(subcommand)]
        command: AutostartCommand,
    },
    Run,
}

#[derive(Subcommand, Debug)]
enum AutostartCommand {
    Install,
    Uninstall,
    Status,
}

#[derive(Debug, Clone)]
enum UserEvent {
    ShowSettings,
    ShowPermissions,
    ShowAbout,
    RefreshPermissions,
    RefreshClientId,
    Reconnect,
    Quit,
    Saved,
    Registered(u64, serde_json::Value),
    RuntimeStatusUpdated(u64, RuntimeStatus),
    RuntimeError(u64, String),
}

#[derive(Clone)]
struct SharedState {
    config_path: std::path::PathBuf,
    config: Arc<Mutex<NodeConfig>>,
    status: Arc<Mutex<RuntimeStatus>>,
    generation: Arc<Mutex<u64>>,
}

struct TrayMenuItems {
    status: MenuItem,
    latency: MenuItem,
    binding: MenuItem,
    client_name: MenuItem,
    client_id: MenuItem,
    device_id: MenuItem,
    server_url: MenuItem,
    browser_preference_auto: CheckMenuItem,
    browser_preference_edge: CheckMenuItem,
    browser_preference_chrome: CheckMenuItem,
    browser_mode_headless: CheckMenuItem,
    browser_mode_headed: CheckMenuItem,
    auto_connect_on: CheckMenuItem,
    auto_connect_off: CheckMenuItem,
    log_level_info: CheckMenuItem,
    log_level_debug: CheckMenuItem,
    log_level_warn: CheckMenuItem,
    log_level_error: CheckMenuItem,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = node_paths()?;
    let config_path = cli.config.unwrap_or(paths.config_file.clone());
    let mut config = NodeConfig::load_or_create(&config_path)?;
    init_logging(&config.log_level);

    match cli.command.unwrap_or(Command::Run) {
        Command::Register => {
            let registered = ensure_registration(&mut config).await?;
            config.save(&config_path)?;
            println!("{}", serde_json::to_string_pretty(&registered)?);
            Ok(())
        }
        Command::Status => {
            let status = status_snapshot(&config);
            println!("{}", serde_json::to_string_pretty(&status)?);
            Ok(())
        }
        Command::Autostart { command } => {
            match command {
                AutostartCommand::Install => {
                    let exe = std::env::current_exe()?;
                    let path = install_launch_agent(&exe)?;
                    println!("installed autostart: {}", path.display());
                }
                AutostartCommand::Uninstall => {
                    uninstall_launch_agent()?;
                    println!("autostart removed");
                }
                AutostartCommand::Status => {
                    println!(
                        "{}",
                        if launch_agent_installed()? { "installed" } else { "not installed" }
                    );
                }
            }
            Ok(())
        }
        Command::Run => run_tray(config_path, config),
    }
}

fn run_tray(config_path: std::path::PathBuf, config: NodeConfig) -> Result<()> {
    let mut event_loop: EventLoop<UserEvent> = EventLoopBuilder::with_user_event().build();
    set_accessory_activation_policy(&mut event_loop);
    let proxy = event_loop.create_proxy();

    let state = SharedState {
        config_path,
        config: Arc::new(Mutex::new(config.clone())),
        status: Arc::new(Mutex::new(status_snapshot(&config))),
        generation: Arc::new(Mutex::new(0)),
    };

    let (menu, items) = build_menu(&state)?;
    let _tray = TrayIconBuilder::new()
        .with_tooltip("Dux AI Node")
        .with_menu(Box::new(menu))
        .with_icon(build_icon()?)
        .with_icon_as_template(cfg!(target_os = "macos"))
        .build()
        .context("failed to create tray icon")?;

    let mut settings_window: Option<Window> = None;
    let mut settings_webview: Option<wry::WebView> = None;
    let mut permissions_window: Option<Window> = None;
    let mut permissions_webview: Option<wry::WebView> = None;
    let mut about_window: Option<Window> = None;
    let mut about_webview: Option<wry::WebView> = None;
    #[cfg(target_os = "windows")]
    let mut settings_context = Some(WebContext::new(Some(webview_data_dir("settings"))));
    #[cfg(target_os = "windows")]
    let mut permissions_context = Some(WebContext::new(Some(webview_data_dir("permissions"))));
    #[cfg(target_os = "windows")]
    let mut about_context = Some(WebContext::new(Some(webview_data_dir("about"))));
    let mut runtime_tasks: Option<(JoinHandle<()>, JoinHandle<()>)> = None;

    let menu_channel = MenuEvent::receiver();

    spawn_runtime_task(&state, &proxy, &mut runtime_tasks);

    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::Wait;

        while let Ok(menu_event) = menu_channel.try_recv() {
            match menu_event.id.as_ref() {
                "action.settings" => {
                    let _ = proxy.send_event(UserEvent::ShowSettings);
                }
                "action.permissions" => {
                    let _ = proxy.send_event(UserEvent::ShowPermissions);
                }
                "action.reconnect" => {
                    let _ = proxy.send_event(UserEvent::Reconnect);
                }
                "action.quit" => {
                    let _ = proxy.send_event(UserEvent::Quit);
                }
                "action.open_node_log" => {
                    let _ = open_in_system(&resolve_log_files().node_log);
                }
                "action.open_connection_log" => {
                    let _ = open_in_system(&resolve_log_files().connection_log);
                }
                "action.open_log_dir" => {
                    let _ = open_in_system(&resolve_log_files().log_dir);
                }
                "action.clear_logs" => {
                    let _ = clear_log_files();
                }
                "action.about" => {
                    let _ = proxy.send_event(UserEvent::ShowAbout);
                }
                "config.browser_preference.auto" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "browser_preference",
                        "auto",
                    );
                }
                "config.browser_preference.msedge" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "browser_preference",
                        "msedge",
                    );
                }
                "config.browser_preference.chrome" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "browser_preference",
                        "chrome",
                    );
                }
                "config.browser_mode.headless" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "browser_mode",
                        "headless",
                    );
                }
                "config.browser_mode.headed" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "browser_mode",
                        "headed",
                    );
                }
                "config.auto_connect.on" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "auto_connect",
                        "true",
                    );
                }
                "config.auto_connect.off" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "auto_connect",
                        "false",
                    );
                }
                "config.log_level.info" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "log_level",
                        "info",
                    );
                }
                "config.log_level.debug" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "log_level",
                        "debug",
                    );
                }
                "config.log_level.warn" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "log_level",
                        "warn",
                    );
                }
                "config.log_level.error" => {
                    apply_menu_config_change(
                        &state,
                        &proxy,
                        &mut runtime_tasks,
                        &items,
                        "log_level",
                        "error",
                    );
                }
                "node.client_name" => copy_menu_value(&state, |config, status| {
                    if !config.client_name.trim().is_empty() {
                        config.client_name.clone()
                    } else {
                        status.client_name.clone()
                    }
                }),
                "node.client_id" => copy_menu_value(&state, |config, status| {
                    if !config.client_id.trim().is_empty() {
                        config.client_id.clone()
                    } else {
                        status.client_id.clone()
                    }
                }),
                "node.device_id" => copy_menu_value(&state, |config, _| config.device_id.clone()),
                "node.server_url" => copy_menu_value(&state, |config, _| config.server_url.clone()),
                _ => {}
            }
        }

        match event {
            Event::NewEvents(StartCause::Init) => {
                set_accessory_activation_policy_runtime(event_loop);
            }
            Event::UserEvent(UserEvent::ShowSettings) => {
                if settings_webview.is_none() {
                    let window = WindowBuilder::new()
                        .with_title("Dux AI Node 设置")
                        .with_inner_size(LogicalSize::new(720.0, 620.0))
                        .with_visible(true)
                        .build(event_loop)
                        .expect("failed to build settings window");

                    let html = render_settings_html(&state);
                    let nav_state = state.clone();
                    let nav_proxy = proxy.clone();
                    #[cfg(not(target_os = "windows"))]
                    let builder = WebViewBuilder::new(&window);
                    #[cfg(target_os = "windows")]
                    let mut builder = WebViewBuilder::new(&window);
                    #[cfg(target_os = "windows")]
                    {
                        if let Some(context) = settings_context.as_mut() {
                            builder = builder.with_web_context(context);
                        }
                    }
                    let webview = builder
                        .with_navigation_handler(move |url| {
                            handle_navigation(&nav_state, &nav_proxy, url)
                        })
                        .with_html(&html)
                        .build()
                        .expect("failed to build settings webview");

                    settings_window = Some(window);
                    settings_webview = Some(webview);
                } else if let Some(window) = &settings_window {
                    let _ = window.set_visible(true);
                    window.set_focus();
                }
            }
            Event::UserEvent(UserEvent::ShowPermissions) => {
                #[cfg(target_os = "windows")]
                {
                    // Windows does not use the macOS-style permissions guide window.
                }
                #[cfg(not(target_os = "windows"))]
                {
                    if permissions_webview.is_some() {
                        drop(permissions_webview.take());
                        drop(permissions_window.take());
                    }
                    if permissions_webview.is_none() {
                        let window = WindowBuilder::new()
                            .with_title("Dux AI Node 权限")
                            .with_inner_size(LogicalSize::new(760.0, 560.0))
                            .with_visible(true)
                            .build(event_loop)
                            .expect("failed to build permissions window");

                        let html = render_permissions_html();
                        let nav_state = state.clone();
                        let nav_proxy = proxy.clone();
                        let builder = WebViewBuilder::new(&window);
                        let webview = builder
                            .with_navigation_handler(move |url| {
                                handle_navigation(&nav_state, &nav_proxy, url)
                            })
                            .with_html(&html)
                            .build()
                            .expect("failed to build permissions webview");

                        permissions_window = Some(window);
                        permissions_webview = Some(webview);
                    } else if let Some(window) = &permissions_window {
                        let _ = window.set_visible(true);
                        window.set_focus();
                    }
                }
            }
            Event::UserEvent(UserEvent::RefreshPermissions) => {
                let _ = proxy.send_event(UserEvent::ShowPermissions);
            }
            Event::UserEvent(UserEvent::ShowAbout) => {
                if about_webview.is_none() {
                    let window = WindowBuilder::new()
                        .with_title("关于 Dux AI Node")
                        .with_inner_size(LogicalSize::new(640.0, 500.0))
                        .with_visible(true)
                        .build(event_loop)
                        .expect("failed to build about window");

                    let nav_state = state.clone();
                    let nav_proxy = proxy.clone();
                    #[cfg(not(target_os = "windows"))]
                    let builder = WebViewBuilder::new(&window);
                    #[cfg(target_os = "windows")]
                    let mut builder = WebViewBuilder::new(&window);
                    #[cfg(target_os = "windows")]
                    {
                        if let Some(context) = about_context.as_mut() {
                            builder = builder.with_web_context(context);
                        }
                    }
                    let webview = builder
                        .with_navigation_handler(move |url| {
                            handle_navigation(&nav_state, &nav_proxy, url)
                        })
                        .with_html(&render_about_html())
                        .build()
                        .expect("failed to build about webview");

                    about_window = Some(window);
                    about_webview = Some(webview);
                } else if let Some(window) = &about_window {
                    let _ = window.set_visible(true);
                    window.set_focus();
                }
            }
            Event::UserEvent(UserEvent::RefreshClientId) => {
                if let Ok(mut guard) = state.config.lock() {
                    guard.refresh_client_id();
                    let _ = guard.save(&state.config_path);
                }
                spawn_runtime_task(&state, &proxy, &mut runtime_tasks);
                let _ = refresh_menu_labels(&items, &state);
                if let Some(window) = &settings_window {
                    let _ = window.set_title("Dux AI Node 设置（连接 ID 已刷新）");
                }
            }
            Event::UserEvent(UserEvent::Reconnect) => {
                if let Ok(mut status) = state.status.lock() {
                    status.connected = false;
                    status.latency_ms = None;
                }
                spawn_runtime_task(&state, &proxy, &mut runtime_tasks);
                let _ = refresh_menu_labels(&items, &state);
            }
            Event::UserEvent(UserEvent::Registered(generation, payload)) => {
                if current_generation(&state) != generation {
                    return;
                }
                if let Ok(mut guard) = state.config.lock() {
                    if let Some(client_id) =
                        payload.get("client_id").and_then(serde_json::Value::as_str)
                    {
                        guard.client_id = client_id.to_string();
                    }
                    if let Some(token) = payload.get("token").and_then(serde_json::Value::as_str) {
                        guard.node_token = token.to_string();
                    }
                    if let Some(name) = payload.get("name").and_then(serde_json::Value::as_str) {
                        guard.client_name = name.to_string();
                    }
                    let _ = guard.save(&state.config_path);
                }
                let _ = refresh_menu_labels(&items, &state);
                if let Some(window) = &settings_window {
                    let _ = window.set_title("Dux AI Node 设置（已注册）");
                }
            }
            Event::UserEvent(UserEvent::RuntimeStatusUpdated(generation, mut next_status)) => {
                if current_generation(&state) != generation {
                    return;
                }
                if let Ok(mut status) = state.status.lock() {
                    if next_status.bound_session.session_id.is_none()
                        && next_status.bound_session.session_title.is_none()
                        && (status.bound_session.session_id.is_some()
                            || status.bound_session.session_title.is_some())
                    {
                        next_status.bound_session = status.bound_session.clone();
                    }
                    *status = next_status;
                }
                let _ = refresh_menu_labels(&items, &state);
            }
            Event::UserEvent(UserEvent::RuntimeError(generation, message)) => {
                if current_generation(&state) != generation {
                    return;
                }
                if let Ok(mut status) = state.status.lock() {
                    status.connected = false;
                    status.latency_ms = None;
                }
                if message.starts_with("permission_required:") {
                    let _ = proxy.send_event(UserEvent::ShowPermissions);
                }
                let _ = refresh_menu_labels(&items, &state);
            }
            Event::UserEvent(UserEvent::Saved) => {
                spawn_runtime_task(&state, &proxy, &mut runtime_tasks);
                let _ = refresh_menu_labels(&items, &state);
                if let Some(window) = &settings_window {
                    let _ = window.set_title("Dux AI Node 设置（已保存）");
                }
            }
            Event::UserEvent(UserEvent::Quit) => {
                if let Ok(config) = state.config.lock() {
                    shutdown_browser_runtime(&config);
                }
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent { window_id, event, .. } => match event {
                WindowEvent::CloseRequested => {
                    if let Some(window) = &settings_window {
                        if window.id() == window_id {
                            let _ = window.set_visible(false);
                        }
                    }
                    if let Some(window) = &permissions_window {
                        if window.id() == window_id {
                            let _ = window.set_visible(false);
                        }
                    }
                    if let Some(window) = &about_window {
                        if window.id() == window_id {
                            let _ = window.set_visible(false);
                        }
                    }
                }
                _ => {}
            },
            Event::LoopDestroyed => {
                if let Ok(config) = state.config.lock() {
                    shutdown_browser_runtime(&config);
                }
                if let Some((forwarder, runner)) = runtime_tasks.take() {
                    forwarder.abort();
                    runner.abort();
                }
                drop(settings_webview.take());
                drop(settings_window.take());
                drop(permissions_webview.take());
                drop(permissions_window.take());
                drop(about_webview.take());
                drop(about_window.take());
            }
            _ => {}
        }
    });
}

fn spawn_runtime_task(
    state: &SharedState,
    proxy: &EventLoopProxy<UserEvent>,
    runtime_tasks: &mut Option<(JoinHandle<()>, JoinHandle<()>)>,
) {
    if let Some((forwarder, runner)) = runtime_tasks.take() {
        forwarder.abort();
        runner.abort();
    }

    let generation = {
        let mut guard = state.generation.lock().expect("generation lock");
        *guard += 1;
        *guard
    };

    let (runtime_tx, mut runtime_rx) = tokio::sync::mpsc::unbounded_channel::<RuntimeUpdate>();
    let event_proxy = proxy.clone();
    let runtime_state = state.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(event) = runtime_rx.recv().await {
            match event {
                RuntimeUpdate::Registered(device) => {
                    let payload = serde_json::to_value(device).unwrap_or_default();
                    let _ = event_proxy.send_event(UserEvent::Registered(generation, payload));
                }
                RuntimeUpdate::Status(status) => {
                    let _ =
                        event_proxy.send_event(UserEvent::RuntimeStatusUpdated(generation, status));
                }
                RuntimeUpdate::Error(message) => {
                    let _ = event_proxy.send_event(UserEvent::RuntimeError(generation, message));
                }
                RuntimeUpdate::ActionRequest(event) => {
                    let state = runtime_state.clone();
                    let error_proxy = event_proxy.clone();
                    tokio::spawn(async move {
                        let config =
                            state.config.lock().map(|item| item.clone()).unwrap_or_default();
                        let outcome = execute_action(&config, &event.action, event.payload.clone());
                        match outcome {
                            Ok(response) => {
                                let _ = publish_action_result(
                                    &config,
                                    &event.request_id,
                                    "completed",
                                    response.result,
                                    response.artifacts,
                                    None,
                                )
                                .await;
                            }
                            Err(error) => {
                                let _ = publish_action_result(
                                    &config,
                                    &event.request_id,
                                    "failed",
                                    serde_json::json!({}),
                                    vec![],
                                    Some(error.to_string()),
                                )
                                .await;
                                let _ = error_proxy.send_event(UserEvent::RuntimeError(
                                    generation,
                                    error.to_string(),
                                ));
                            }
                        }
                    });
                }
            }
        }
    });

    let state = state.clone();
    let runner = tokio::spawn(async move {
        let config = state.config.lock().map(|item| item.clone()).unwrap_or_default();
        let _ = run_runtime_with_updates(config, Some(runtime_tx)).await;
    });

    *runtime_tasks = Some((forwarder, runner));
}

fn current_generation(state: &SharedState) -> u64 {
    state.generation.lock().map(|item| *item).unwrap_or_default()
}

fn apply_menu_config_change(
    state: &SharedState,
    proxy: &EventLoopProxy<UserEvent>,
    runtime_tasks: &mut Option<(JoinHandle<()>, JoinHandle<()>)>,
    items: &TrayMenuItems,
    key: &str,
    value: &str,
) {
    let mut restart_browser_runtime = false;
    let mut next_config = None;
    if let Ok(mut config) = state.config.lock() {
        let unchanged = match key {
            "browser_preference" => config.browser_preference == value,
            "browser_mode" => config.browser_mode == value,
            "auto_connect" => config.auto_connect == matches!(value, "1" | "true" | "yes" | "on"),
            "log_level" => config.log_level == value,
            _ => false,
        };
        if unchanged {
            return;
        }
        let _ = config.set_value(key, value);
        let _ = config.save(&state.config_path);
        restart_browser_runtime = matches!(key, "browser_preference" | "browser_mode");
        if restart_browser_runtime {
            next_config = Some(config.clone());
        }
    }
    if restart_browser_runtime {
        if let Some(config) = next_config.as_ref() {
            shutdown_browser_runtime(config);
        }
    }
    spawn_runtime_task(state, proxy, runtime_tasks);
    let _ = refresh_menu_labels(items, state);
}

fn copy_menu_value<F>(state: &SharedState, getter: F)
where
    F: Fn(&NodeConfig, &RuntimeStatus) -> String,
{
    if let (Ok(config), Ok(status)) = (state.config.lock(), state.status.lock()) {
        let value = getter(&config, &status);
        if !value.trim().is_empty() {
            let _ = copy_to_clipboard(&value);
        }
    }
}

fn open_in_system(path: &std::path::Path) -> Result<()> {
    if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).spawn().context("failed to open path")?;
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .context("failed to open path")?;
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn().context("failed to open path")?;
    }
    Ok(())
}

fn open_url(url: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn().context("failed to open url")?;
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .context("failed to open url")?;
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn().context("failed to open url")?;
    }
    Ok(())
}

fn build_menu(state: &SharedState) -> Result<(Menu, TrayMenuItems)> {
    let menu = Menu::new();
    let snapshot = menu_snapshot(state);

    let status = MenuItem::with_id(
        "status.connection",
        format!("连接状态: {}", snapshot.status_text),
        false,
        None,
    );
    let latency = MenuItem::with_id(
        "status.latency",
        format!("连接延迟: {}", snapshot.latency_text),
        false,
        None,
    );
    let binding = MenuItem::with_id(
        "status.binding",
        format!("绑定状态: {}", snapshot.binding_text),
        false,
        None,
    );

    let browser_preference_auto =
        CheckMenuItem::with_id("config.browser_preference.auto", "自动", true, false, None);
    let browser_preference_edge =
        CheckMenuItem::with_id("config.browser_preference.msedge", "edge", true, false, None);
    let browser_preference_chrome =
        CheckMenuItem::with_id("config.browser_preference.chrome", "chrome", true, false, None);
    let browser_preference = Submenu::with_items(
        "浏览器偏好",
        true,
        &[&browser_preference_auto, &browser_preference_edge, &browser_preference_chrome],
    )?;

    let browser_mode_headless =
        CheckMenuItem::with_id("config.browser_mode.headless", "无头", true, false, None);
    let browser_mode_headed =
        CheckMenuItem::with_id("config.browser_mode.headed", "有界面", true, false, None);
    let browser_mode =
        Submenu::with_items("浏览器模式", true, &[&browser_mode_headless, &browser_mode_headed])?;

    let auto_connect_on =
        CheckMenuItem::with_id("config.auto_connect.on", "开启", true, false, None);
    let auto_connect_off =
        CheckMenuItem::with_id("config.auto_connect.off", "关闭", true, false, None);
    let auto_connect =
        Submenu::with_items("自动连接", true, &[&auto_connect_on, &auto_connect_off])?;

    let log_level_info = CheckMenuItem::with_id("config.log_level.info", "info", true, false, None);
    let log_level_debug =
        CheckMenuItem::with_id("config.log_level.debug", "debug", true, false, None);
    let log_level_warn = CheckMenuItem::with_id("config.log_level.warn", "warn", true, false, None);
    let log_level_error =
        CheckMenuItem::with_id("config.log_level.error", "error", true, false, None);
    let log_level = Submenu::with_items(
        "日志等级",
        true,
        &[&log_level_info, &log_level_debug, &log_level_warn, &log_level_error],
    )?;

    let client_name = MenuItem::with_id(
        "node.client_name",
        format!("节点名称: {}", snapshot.client_name),
        true,
        None,
    );
    let client_id =
        MenuItem::with_id("node.client_id", format!("连接 ID: {}", snapshot.client_id), true, None);
    let device_id =
        MenuItem::with_id("node.device_id", format!("设备 ID: {}", snapshot.device_id), true, None);
    let server_url = MenuItem::with_id(
        "node.server_url",
        format!("服务器地址: {}", snapshot.server_url),
        true,
        None,
    );
    let reconnect = MenuItem::with_id("action.reconnect", "重新连接", true, None);
    let node_info = Submenu::with_items(
        "节点信息",
        true,
        &[
            &client_name,
            &client_id,
            &device_id,
            &server_url,
            &PredefinedMenuItem::separator(),
            &reconnect,
        ],
    )?;

    let open_node_log = MenuItem::with_id("action.open_node_log", "节点日志", true, None);
    let open_connection_log =
        MenuItem::with_id("action.open_connection_log", "连接日志", true, None);
    let open_log_dir = MenuItem::with_id("action.open_log_dir", "日志目录", true, None);
    let logs =
        Submenu::with_items("日志", true, &[&open_node_log, &open_connection_log, &open_log_dir])?;

    let about = MenuItem::with_id("action.about", "关于", true, None);

    let settings = MenuItem::with_id("action.settings", "设置", true, None);
    let quit = MenuItem::with_id("action.quit", "退出", true, None);

    menu.append(&status)?;
    menu.append(&latency)?;
    menu.append(&binding)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&browser_preference)?;
    menu.append(&browser_mode)?;
    menu.append(&auto_connect)?;
    menu.append(&log_level)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&node_info)?;
    menu.append(&logs)?;
    #[cfg(target_os = "macos")]
    {
        let permissions = MenuItem::with_id("action.permissions", "权限引导", true, None);
        menu.append(&permissions)?;
    }
    menu.append(&settings)?;
    menu.append(&about)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit)?;

    let items = TrayMenuItems {
        status,
        latency,
        binding,
        client_name,
        client_id,
        device_id,
        server_url,
        browser_preference_auto,
        browser_preference_edge,
        browser_preference_chrome,
        browser_mode_headless,
        browser_mode_headed,
        auto_connect_on,
        auto_connect_off,
        log_level_info,
        log_level_debug,
        log_level_warn,
        log_level_error,
    };
    refresh_menu_labels(&items, state)?;
    Ok((menu, items))
}

fn refresh_menu_labels(items: &TrayMenuItems, state: &SharedState) -> Result<()> {
    let snapshot = menu_snapshot(state);
    items.status.set_text(format!("连接状态: {}", snapshot.status_text));
    items.latency.set_text(format!("连接延迟: {}", snapshot.latency_text));
    items.binding.set_text(format!("绑定状态: {}", snapshot.binding_text));
    items.client_name.set_text(format!("节点名称: {}", snapshot.client_name));
    items.client_id.set_text(format!("连接 ID: {}", snapshot.client_id));
    items.device_id.set_text(format!("设备 ID: {}", snapshot.device_id));
    items.server_url.set_text(format!("服务器地址: {}", snapshot.server_url));

    items.browser_preference_auto.set_checked(snapshot.browser_preference == "auto");
    items.browser_preference_edge.set_checked(snapshot.browser_preference == "msedge");
    items.browser_preference_chrome.set_checked(snapshot.browser_preference == "chrome");
    items.browser_mode_headless.set_checked(snapshot.browser_mode == "headless");
    items.browser_mode_headed.set_checked(snapshot.browser_mode == "headed");
    items.auto_connect_on.set_checked(snapshot.auto_connect);
    items.auto_connect_off.set_checked(!snapshot.auto_connect);
    items.log_level_info.set_checked(snapshot.log_level == "info");
    items.log_level_debug.set_checked(snapshot.log_level == "debug");
    items.log_level_warn.set_checked(snapshot.log_level == "warn");
    items.log_level_error.set_checked(snapshot.log_level == "error");
    Ok(())
}

struct MenuSnapshot {
    status_text: String,
    latency_text: String,
    binding_text: String,
    client_name: String,
    client_id: String,
    device_id: String,
    server_url: String,
    browser_preference: String,
    browser_mode: String,
    auto_connect: bool,
    log_level: String,
}

fn menu_snapshot(state: &SharedState) -> MenuSnapshot {
    let config = state.config.lock().map(|item| item.clone()).unwrap_or_default();
    let status = state.status.lock().map(|item| item.clone()).unwrap_or_default();
    let status_text =
        if status.connected { "已连接".to_string() } else { "未连接".to_string() };
    let latency_text =
        status.latency_ms.map(|item| format!("{} ms", item)).unwrap_or_else(|| "未知".to_string());
    let binding_text = format_bound_session(&status.bound_session);
    let client_name = if config.client_name.trim().is_empty() {
        if status.client_name.trim().is_empty() {
            "未知".to_string()
        } else {
            status.client_name
        }
    } else {
        config.client_name.clone()
    };
    let client_id = if config.client_id.trim().is_empty() {
        if status.client_id.trim().is_empty() {
            "未分配".to_string()
        } else {
            status.client_id
        }
    } else {
        config.client_id.clone()
    };

    MenuSnapshot {
        status_text,
        latency_text,
        binding_text,
        client_name,
        client_id,
        device_id: if config.device_id.trim().is_empty() {
            "未知".to_string()
        } else {
            config.device_id.clone()
        },
        server_url: if config.server_url.trim().is_empty() {
            "未设置".to_string()
        } else {
            config.server_url.clone()
        },
        browser_preference: config.browser_preference.clone(),
        browser_mode: config.browser_mode.clone(),
        auto_connect: config.auto_connect,
        log_level: config.log_level.clone(),
    }
}

fn format_bound_session(bound: &BoundSessionState) -> String {
    match (bound.session_id, bound.session_title.as_deref()) {
        (Some(id), Some(title)) if !title.trim().is_empty() => format!("{} (#{} )", title, id),
        (Some(id), _) => format!("会话 #{}", id),
        _ => "未绑定".to_string(),
    }
}

fn build_icon() -> Result<Icon> {
    let png = if cfg!(target_os = "macos") {
        include_bytes!("../../../assets/tray-template.png") as &[u8]
    } else {
        include_bytes!("../../../assets/icon-32.png") as &[u8]
    };
    let image = image::load_from_memory_with_format(png, ImageFormat::Png)
        .context("failed to decode tray icon")?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).context("failed to build tray icon")
}

fn render_settings_html(state: &SharedState) -> String {
    let config = state.config.lock().map(|item| item.clone()).unwrap_or_default();
    let status = state.status.lock().map(|item| item.clone()).unwrap_or_default();
    let browser = browser_runtime(&config);

    render_template(
        "web/settings/templates/settings.html",
        context! {
            app_css => load_settings_resource("web/settings/app.css"),
            server_url => config.server_url,
            client_name => config.client_name,
            client_id => if config.client_id.trim().is_empty() { "" } else { config.client_id.as_str() },
            device_id => config.device_id,
            auto_pref => selected(&config.browser_preference, "auto"),
            chrome_pref => selected(&config.browser_preference, "chrome"),
            edge_pref => selected(&config.browser_preference, "msedge"),
            headless_mode => selected(&config.browser_mode, "headless"),
            headed_mode => selected(&config.browser_mode, "headed"),
            auto_yes => if config.auto_connect { "selected" } else { "" },
            auto_no => if config.auto_connect { "" } else { "selected" },
            log_info => selected(&config.log_level, "info"),
            log_debug => selected(&config.log_level, "debug"),
            log_warn => selected(&config.log_level, "warn"),
            log_error => selected(&config.log_level, "error"),
            connected => if status.connected { "已连接" } else { "未连接" },
            latency => status.latency_ms.map(|item| format!("{} ms", item)).unwrap_or_else(|| "未知".to_string()),
            runtime_mode => browser.runtime_mode,
            browser_backend => browser.browser_backend,
            binding => format_bound_session(&status.bound_session),
        },
    )
}

fn render_permissions_html() -> String {
    let permissions = permission_statuses()
        .into_iter()
        .map(|item| {
            let action = match item.id.as_str() {
                "app_management" => "open-app-management",
                "screen_capture" => "open-screen-capture",
                "accessibility" => "open-accessibility",
                "full_disk_access" => "open-full-disk-access",
                "automation" => "open-automation",
                _ => "",
            };
            let (badge_class, badge_text) = match item.status.as_str() {
                "granted" => ("badge badge-ok", "已授权"),
                "enabled" => ("badge badge-ok", "已启用"),
                "denied" => ("badge badge-warn", "未授权"),
                "entry" => ("badge badge-unknown", "系统入口"),
                "disabled" => ("badge badge-unknown", "未启用"),
                _ => ("badge badge-unknown", "按需触发"),
            };
            context! {
                summary => item.summary,
                detail => item.detail,
                action => action,
                badge_class => badge_class,
                badge_text => badge_text,
            }
        })
        .collect::<Vec<_>>();

    render_template(
        "web/settings/templates/permissions.html",
        context! {
            app_css => load_settings_resource("web/settings/app.css"),
            exec_path => current_application_path(),
            permissions => permissions,
        },
    )
}

fn render_about_html() -> String {
    render_template(
        "web/settings/templates/about.html",
        context! {
            app_css => load_settings_resource("web/settings/app.css"),
            icon_base64 => include_base64_icon(),
        },
    )
}

fn include_base64_icon() -> &'static str {
    include_str!("../../../assets/icon.png.base64")
}

fn load_settings_resource(relative: &str) -> String {
    match relative {
        "web/settings/app.css" => include_str!("../../../web/settings/app.css").to_string(),
        _ => format!("/* embedded resource not found: {} */", html_escape(relative)),
    }
}

fn render_template(relative: &str, ctx: minijinja::Value) -> String {
    let template = match relative {
        "web/settings/templates/settings.html" => include_str!("../../../web/settings/templates/settings.html").to_string(),
        "web/settings/templates/permissions.html" => include_str!("../../../web/settings/templates/permissions.html").to_string(),
        "web/settings/templates/about.html" => include_str!("../../../web/settings/templates/about.html").to_string(),
        _ => format!(
            "<!doctype html><html><body><pre>embedded template not found: {}</pre></body></html>",
            html_escape(relative)
        ),
    };

    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::Html);
    if let Err(error) = env.add_template_owned(relative.to_string(), template) {
        return format!(
            "<!doctype html><html><body><pre>failed to register template {}: {}</pre></body></html>",
            html_escape(relative),
            html_escape(&error.to_string())
        );
    }

    match env.get_template(relative).and_then(|tpl| tpl.render(ctx)) {
        Ok(html) => html,
        Err(error) => format!(
            "<!doctype html><html><body><pre>failed to render template {}: {}</pre></body></html>",
            html_escape(relative),
            html_escape(&error.to_string())
        ),
    }
}

#[cfg(target_os = "windows")]
fn webview_data_dir(name: &str) -> PathBuf {
    if let Ok(paths) = node_paths() {
        let dir = paths.data_dir.join("webview").join(name);
        let _ = fs::create_dir_all(&dir);
        return dir;
    }
    let dir = std::env::temp_dir().join("dux-ai-node-webview").join(name);
    let _ = fs::create_dir_all(&dir);
    dir
}

fn selected(current: &str, expected: &str) -> &'static str {
    if current == expected {
        "selected"
    } else {
        ""
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn handle_navigation(state: &SharedState, proxy: &EventLoopProxy<UserEvent>, url: String) -> bool {
    if let Ok(parsed) = Url::parse(&url) {
        if parsed.scheme() == "http" || parsed.scheme() == "https" {
            let _ = open_url(parsed.as_str());
            return false;
        }
        if parsed.scheme() == "dux" {
            match parsed.host_str().unwrap_or_default() {
                "save" => {
                    if let Ok(mut guard) = state.config.lock() {
                        for (key, value) in parsed.query_pairs() {
                            let _ = guard.set_value(key.as_ref(), value.as_ref());
                        }
                        let _ = guard.save(&state.config_path);
                    }
                    let _ = proxy.send_event(UserEvent::Saved);
                }
                "refresh-client-id" => {
                    let _ = proxy.send_event(UserEvent::RefreshClientId);
                }
                "open-permissions" => {
                    let _ = proxy.send_event(UserEvent::ShowPermissions);
                }
                "refresh-permissions" => {
                    let _ = proxy.send_event(UserEvent::RefreshPermissions);
                }
                "open-privacy-settings" => {
                    let _ = open_privacy_settings();
                }
                "open-app-management" => {
                    let _ = open_permission_settings("app_management");
                }
                "copy-executable-path" => {
                    let _ = copy_to_clipboard(&current_application_path());
                }
                "restart-app" => {
                    let _ = relaunch_current_application();
                    let _ = proxy.send_event(UserEvent::Quit);
                }
                "open-screen-capture" => {
                    let _ = open_permission_settings("screen_capture");
                }
                "open-accessibility" => {
                    let _ = open_permission_settings("accessibility");
                }
                "open-full-disk-access" => {
                    let _ = open_permission_settings("full_disk_access");
                }
                "open-automation" => {
                    let _ = open_permission_settings("automation");
                }
                _ => {}
            }
            return false;
        }
    }
    true
}

fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let mut child = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("failed to spawn pbcopy")?;
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            stdin.write_all(text.as_bytes()).context("failed to write to pbcopy")?;
        }
        let _ = child.wait();
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        let mut child = std::process::Command::new("clip")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("failed to spawn clip")?;
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            stdin.write_all(text.as_bytes()).context("failed to write to clip")?;
        }
        let _ = child.wait();
        return Ok(());
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = text;
        anyhow::bail!("clipboard copy is not implemented for this platform")
    }
}

#[cfg(target_os = "macos")]
fn set_accessory_activation_policy(event_loop: &mut EventLoop<UserEvent>) {
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    event_loop.set_activate_ignoring_other_apps(false);
}

#[cfg(not(target_os = "macos"))]
fn set_accessory_activation_policy(_event_loop: &mut EventLoop<UserEvent>) {}

#[cfg(target_os = "macos")]
fn set_accessory_activation_policy_runtime(
    event_loop: &tao::event_loop::EventLoopWindowTarget<UserEvent>,
) {
    event_loop.set_activation_policy_at_runtime(ActivationPolicy::Accessory);
}

#[cfg(not(target_os = "macos"))]
fn set_accessory_activation_policy_runtime(
    _event_loop: &tao::event_loop::EventLoopWindowTarget<UserEvent>,
) {
}
