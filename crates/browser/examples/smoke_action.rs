use dux_ai_node_browser::execute_action;
use dux_ai_node_core::node_paths;
use serde_json::{json, Value};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let action = args.next().unwrap_or_else(|| "system.info".to_string());
    let payload = args
        .next()
        .map(|item| serde_json::from_str::<Value>(&item))
        .transpose()?
        .unwrap_or_else(|| json!({}));

    let paths = node_paths()?;
    let config = dux_ai_node_core::NodeConfig::load_or_create(&paths.config_file)?;
    let response = execute_action(&config, &action, payload)?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
