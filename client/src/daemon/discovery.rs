#![allow(dead_code)]

// Reads ~/.pixel-agents/daemon.json to locate the running daemon.

use anyhow::{bail, Result};
use directories::BaseDirs;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct DaemonDiscovery {
    #[serde(rename = "socketPath")]
    pub socket_path: String,
    pub token: String,
    #[serde(rename = "bootId")]
    pub boot_id: String,
    pub pid: u32,
    pub version: String,
    #[serde(rename = "hookPort", skip_serializing_if = "Option::is_none")]
    pub hook_port: Option<u16>,
}

pub fn daemon_json_path() -> Result<PathBuf> {
    let base = BaseDirs::new().ok_or_else(|| anyhow::anyhow!("cannot locate home directory"))?;
    Ok(base.home_dir().join(".pixel-agents").join("daemon.json"))
}

pub fn read_discovery() -> Result<DaemonDiscovery> {
    let path = daemon_json_path()?;
    if !path.exists() {
        bail!("daemon.json not found at {} — is the daemon running?", path.display());
    }
    let raw = std::fs::read_to_string(&path)?;
    let d: DaemonDiscovery = serde_json::from_str(&raw)?;
    Ok(d)
}
