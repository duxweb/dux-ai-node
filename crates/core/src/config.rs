use anyhow::Context;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeConfig {
    pub server_url: String,
    pub token: String,
    pub device_id: String,
    pub client_id: String,
    pub client_name: String,
    pub browser_preference: String,
    pub browser_mode: String,
    pub auto_connect: bool,
    pub log_level: String,
    pub node_token: String,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            token: String::new(),
            device_id: Uuid::now_v7().to_string(),
            client_id: String::new(),
            client_name: default_client_name(),
            browser_preference: "auto".to_string(),
            browser_mode: "headless".to_string(),
            auto_connect: true,
            log_level: "info".to_string(),
            node_token: String::new(),
        }
    }
}

impl NodeConfig {
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            return Self::load(path);
        }
        let config = Self::default();
        config.save(path)?;
        Ok(config)
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config from {}", path.display()))?;
        let mut config = toml::from_str::<Self>(&raw)
            .with_context(|| format!("failed to parse config from {}", path.display()))?;
        if config.device_id.trim().is_empty() {
            config.device_id = if config.client_id.trim().is_empty() {
                Uuid::now_v7().to_string()
            } else {
                config.client_id.clone()
            };
        }
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create config dir {}", parent.display()))?;
        }
        let raw = toml::to_string_pretty(self).context("failed to serialize node config")?;
        fs::write(path, raw)
            .with_context(|| format!("failed to write config to {}", path.display()))?;
        Ok(())
    }

    pub fn refresh_client_id(&mut self) {
        self.client_id.clear();
        self.node_token.clear();
    }

    pub fn set_value(&mut self, key: &str, value: &str) -> anyhow::Result<()> {
        match key {
            "server_url" => self.server_url = value.trim().to_string(),
            "token" => self.token = value.trim().to_string(),
            "device_id" => self.device_id = value.trim().to_string(),
            "client_name" => self.client_name = value.trim().to_string(),
            "browser_preference" => self.browser_preference = value.trim().to_string(),
            "browser_mode" => self.browser_mode = value.trim().to_string(),
            "auto_connect" => {
                self.auto_connect = matches!(value.trim(), "1" | "true" | "yes" | "on")
            }
            "log_level" => self.log_level = value.trim().to_string(),
            "client_id" => self.client_id = value.trim().to_string(),
            "node_token" => self.node_token = value.trim().to_string(),
            _ => anyhow::bail!("unsupported config key: {}", key),
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct NodePaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub config_file: PathBuf,
}

pub fn node_paths() -> anyhow::Result<NodePaths> {
    let dirs = ProjectDirs::from("plus", "dux", "dux-ai-node")
        .ok_or_else(|| anyhow::anyhow!("unable to resolve project directories"))?;
    let config_dir = dirs.config_dir().to_path_buf();
    let data_dir = dirs.data_dir().to_path_buf();
    let log_dir = dirs.data_local_dir().join("logs");
    let config_file = config_dir.join("config.toml");
    Ok(NodePaths { config_dir, data_dir, log_dir, config_file })
}

pub fn default_client_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Dux AI Node".to_string())
}
