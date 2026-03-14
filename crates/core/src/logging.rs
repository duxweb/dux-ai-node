use crate::node_paths;
use once_cell::sync::OnceCell;
use std::fs;
use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::{filter_fn, EnvFilter};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

static LOG_GUARDS: OnceCell<Vec<WorkerGuard>> = OnceCell::new();

#[derive(Debug, Clone)]
pub struct LogFiles {
    pub log_dir: PathBuf,
    pub node_log: PathBuf,
    pub connection_log: PathBuf,
}

pub fn resolve_log_files() -> LogFiles {
    let log_dir = node_paths()
        .map(|paths| paths.log_dir)
        .unwrap_or_else(|_| std::env::temp_dir().join("dux-ai-node/logs"));
    let _ = fs::create_dir_all(&log_dir);

    let node_log = log_dir.join("node.log");
    let connection_log = log_dir.join("connection.log");
    let _ = fs::OpenOptions::new().create(true).append(true).open(&node_log);
    let _ = fs::OpenOptions::new().create(true).append(true).open(&connection_log);

    LogFiles { log_dir, node_log, connection_log }
}

pub fn init_logging(level: &str) {
    let files = resolve_log_files();
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let (node_writer, node_guard) = tracing_appender::non_blocking(
        tracing_appender::rolling::never(&files.log_dir, "node.log"),
    );
    let (conn_writer, conn_guard) = tracing_appender::non_blocking(
        tracing_appender::rolling::never(&files.log_dir, "connection.log"),
    );

    let node_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .compact()
        .with_writer(node_writer)
        .with_filter(filter_fn(|metadata| {
            !metadata.target().starts_with("dux_ai_node_core::runtime")
        }));

    let connection_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .compact()
        .with_writer(conn_writer)
        .with_filter(filter_fn(|metadata| {
            metadata.target().starts_with("dux_ai_node_core::runtime")
        }));

    let stdout_layer =
        tracing_subscriber::fmt::layer().compact().with_target(true).with_filter(filter);

    let _ = tracing_subscriber::registry()
        .with(node_layer)
        .with(connection_layer)
        .with(stdout_layer)
        .try_init();

    let _ = LOG_GUARDS.set(vec![node_guard, conn_guard]);
}
