#![allow(dead_code)]
// Capability cache at ~/.pixel-agents/capabilities-cache.json
// Key = hash of 8 env vars; TTL = 7 days.

use anyhow::Result;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::daemon::RenderingCap;

const CACHE_TTL_SECS: u64 = 7 * 24 * 3600;

/// The 8 environment variables that identify a terminal environment.
/// Changes in any of these invalidate the cached capabilities.
const ENV_KEYS: &[&str] = &[
    "TERM",
    "TERM_PROGRAM",
    "COLORTERM",
    "KITTY_WINDOW_ID",
    "TMUX",
    "ZELLIJ",
    "ITERM_SESSION_ID",
    "VTE_VERSION",
];

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    env_key: String,
    cap: RenderingCap,
    cell_px_w: Option<u16>,
    cell_px_h: Option<u16>,
    saved_at: u64,
}

pub struct CachedCap {
    pub cap: RenderingCap,
    pub cell_px: Option<(u16, u16)>,
}

fn cache_path() -> Option<PathBuf> {
    BaseDirs::new().map(|d| d.home_dir().join(".pixel-agents").join("capabilities-cache.json"))
}

/// Stable key from the 8 env vars. Empty string for unset vars.
pub fn env_key() -> String {
    ENV_KEYS
        .iter()
        .map(|k| std::env::var(k).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("|")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

/// Read cached capability if env_key matches and within TTL.
pub fn read_cache() -> Option<CachedCap> {
    let path = cache_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let entry: CacheFile = serde_json::from_slice(&bytes).ok()?;
    if entry.env_key != env_key() {
        return None;
    }
    if now_secs().saturating_sub(entry.saved_at) > CACHE_TTL_SECS {
        return None;
    }
    Some(CachedCap {
        cap: entry.cap,
        cell_px: match (entry.cell_px_w, entry.cell_px_h) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        },
    })
}

/// Write capability to cache (atomic tmp+rename).
pub fn write_cache(cap: &RenderingCap, cell_px: Option<(u16, u16)>) -> Result<()> {
    let path = cache_path().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let entry = CacheFile {
        env_key: env_key(),
        cap: cap.clone(),
        cell_px_w: cell_px.map(|(w, _)| w),
        cell_px_h: cell_px.map(|(_, h)| h),
        saved_at: now_secs(),
    };
    let json = serde_json::to_vec_pretty(&entry)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_key_stable_across_calls() {
        let k1 = env_key();
        let k2 = env_key();
        assert_eq!(k1, k2);
    }

    #[test]
    fn env_key_untracked_var_irrelevant() {
        let k1 = env_key();
        // __PA_UNTRACKED is not in ENV_KEYS — setting it must not change the key
        unsafe { std::env::set_var("__PA_UNTRACKED", "x") };
        let k2 = env_key();
        assert_eq!(k1, k2);
        unsafe { std::env::remove_var("__PA_UNTRACKED") };
    }

    #[test]
    fn cache_serde_roundtrip() {
        let entry = CacheFile {
            env_key: env_key(),
            cap: RenderingCap::Truecolor,
            cell_px_w: Some(10),
            cell_px_h: Some(20),
            saved_at: now_secs(),
        };
        let json = serde_json::to_vec_pretty(&entry).unwrap();
        let loaded: CacheFile = serde_json::from_slice(&json).unwrap();
        assert_eq!(loaded.env_key, entry.env_key);
        assert_eq!(loaded.cell_px_w, Some(10));
        assert_eq!(loaded.cell_px_h, Some(20));
        assert!(matches!(loaded.cap, RenderingCap::Truecolor));
    }

    #[test]
    fn ttl_epoch_is_expired() {
        assert!(now_secs().saturating_sub(0) > CACHE_TTL_SECS);
    }

    #[test]
    fn ttl_now_is_fresh() {
        assert!(now_secs().saturating_sub(now_secs()) <= CACHE_TTL_SECS);
    }
}
