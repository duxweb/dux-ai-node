use anyhow::Result;
use clap::{Parser, Subcommand};
use dux_ai_node_browser::{
    browser_runtime, execute_action, shutdown_browser_runtime, supported_actions,
};
use dux_ai_node_core::{
    logging::init_logging,
    node_paths,
    runtime::{
        ensure_registration, publish_action_result, run_runtime_with_updates, sample_protocol,
        status_snapshot, RuntimeUpdate,
    },
    NodeConfig,
};
use dux_ai_node_platform::current_platform;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "dux-ai-node", about = "Dux AI node daemon")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Init,
    Register,
    Status,
    SampleProtocol,
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Daemon,
}

#[derive(Subcommand, Debug)]
enum ConfigCommand {
    Get { key: Option<String> },
    Set { key: String, value: String },
    RefreshClientId,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = node_paths()?;
    let config_path = cli.config.unwrap_or(paths.config_file.clone());
    let mut config = NodeConfig::load_or_create(&config_path)?;
    init_logging(&config.log_level);

    match cli.command.unwrap_or(Command::Daemon) {
        Command::Init => {
            config.save(&config_path)?;
            println!("initialized config at {}", config_path.display());
        }
        Command::Register => {
            let registered = ensure_registration(&mut config).await?;
            config.save(&config_path)?;
            println!("{}", serde_json::to_string_pretty(&registered)?);
        }
        Command::Status => {
            let status = status_snapshot(&config);
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        Command::SampleProtocol => {
            let message = sample_protocol("daemon", "linux daemon placeholder is active");
            println!("{}", serde_json::to_string_pretty(&message)?);
        }
        Command::Config { command } => match command {
            ConfigCommand::Get { key } => {
                if let Some(key) = key {
                    let value = match key.as_str() {
                        "server_url" => config.server_url.clone(),
                        "node_token" => config.node_token.clone(),
                        "device_id" => config.device_id.clone(),
                        "client_id" => {
                            if config.client_id.is_empty() {
                                config.device_id.clone()
                            } else {
                                config.client_id.clone()
                            }
                        }
                        "client_name" => config.client_name.clone(),
                        "browser_preference" => config.browser_preference.clone(),
                        "browser_mode" => config.browser_mode.clone(),
                        "auto_connect" => config.auto_connect.to_string(),
                        "log_level" => config.log_level.clone(),
                        _ => anyhow::bail!("unsupported config key: {}", key),
                    };
                    println!("{}", value);
                } else {
                    println!("{}", toml::to_string_pretty(&config)?);
                }
            }
            ConfigCommand::Set { key, value } => {
                config.set_value(&key, &value)?;
                config.save(&config_path)?;
                println!("updated {}", key);
            }
            ConfigCommand::RefreshClientId => {
                config.refresh_client_id();
                config.save(&config_path)?;
                println!("{}", config.client_id);
            }
        },
        Command::Daemon => {
            let browser = browser_runtime(&config);
            info!(config_path = %config_path.display(), actions = ?supported_actions(), browser_backend = %browser.browser_backend, runtime_mode = %browser.runtime_mode, platform = %current_platform(), "dux-ai-node daemon bootstrap");
            let (tx, mut rx) = mpsc::unbounded_channel::<RuntimeUpdate>();
            let runtime_config = config.clone();
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    match event {
                        RuntimeUpdate::Registered(device) => {
                            info!(device = ?device, "node registered")
                        }
                        RuntimeUpdate::Status(status) => {
                            info!(status = ?status, "runtime status updated")
                        }
                        RuntimeUpdate::Error(error) => info!(error = %error, "runtime error"),
                        RuntimeUpdate::ActionRequest(event) => {
                            let config = runtime_config.clone();
                            info!(request_id = %event.request_id, action = %event.action, "desktop action request received");
                            println!("[node] action request {} {}", event.request_id, event.action);
                            tokio::spawn(async move {
                                let outcome =
                                    execute_action(&config, &event.action, event.payload.clone());
                                match outcome {
                                    Ok(response) => {
                                        info!(request_id = %event.request_id, action = %event.action, "desktop action executed");
                                        println!(
                                            "[node] action completed {} {}",
                                            event.request_id, event.action
                                        );
                                        if let Err(error) = publish_action_result(
                                            &config,
                                            &event.request_id,
                                            "completed",
                                            response.result,
                                            response.artifacts,
                                            None,
                                        )
                                        .await
                                        {
                                            info!(request_id = %event.request_id, error = %error, "desktop action result publish failed");
                                            println!(
                                                "[node] publish failed {} {}",
                                                event.request_id, error
                                            );
                                        } else {
                                            println!(
                                                "[node] result published {}",
                                                event.request_id
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        info!(request_id = %event.request_id, error = %error, "desktop action execution failed");
                                        println!(
                                            "[node] action failed {} {}",
                                            event.request_id, error
                                        );
                                        if let Err(publish_error) = publish_action_result(
                                            &config,
                                            &event.request_id,
                                            "failed",
                                            serde_json::json!({}),
                                            vec![],
                                            Some(error.to_string()),
                                        )
                                        .await
                                        {
                                            info!(request_id = %event.request_id, error = %publish_error, "desktop action failure publish failed");
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
            });
            let shutdown_config = config.clone();
            tokio::select! {
                result = run_runtime_with_updates(config, Some(tx)) => { result?; }
                _ = tokio::signal::ctrl_c() => {
                    println!("[node] received shutdown signal");
                }
            }
            shutdown_browser_runtime(&shutdown_config);
        }
    }

    Ok(())
}
