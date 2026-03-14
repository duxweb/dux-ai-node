mod chromiumoxide;
mod enigo_fallback;

use dux_ai_node_core::NodeConfig;
use serde_json::Value;

use crate::ActionResponse;

pub fn browser_backend_name() -> &'static str {
    chromiumoxide::browser_backend_name()
}

pub fn execute_browser_action(
    config: &NodeConfig,
    action: &str,
    payload: Value,
) -> anyhow::Result<ActionResponse> {
    chromiumoxide::execute_browser_action(config, action, payload)
}

pub fn shutdown_browser_runtime(config: &NodeConfig) {
    chromiumoxide::shutdown_browser_runtime(config);
}

pub fn cleanup_browser_runtime(config: &NodeConfig) {
    chromiumoxide::cleanup_browser_runtime(config);
}
