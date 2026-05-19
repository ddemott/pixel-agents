#![allow(dead_code)]
// Capability detection: probe terminal, apply heuristics, cache result.

mod cache;
mod probe;

use anyhow::Result;
use tokio::io::AsyncWriteExt;
use tokio::time::Duration;

use crate::daemon::{CellPx, ClientCapabilities, RenderingCap};
use crate::raw_mode::RawModeGuard;
use probe::{
    in_multiplexer, is_native_kitty, parse_replies, PROBE_DA1, PROBE_ITERM2, PROBE_KITTY,
    PROBE_PIXEL_SIZE,
};

const PROBE_TIMEOUT: Duration = Duration::from_millis(150);

fn build_caps(cols: u16, rows: u16, rendering: RenderingCap, cell_px: Option<CellPx>) -> ClientCapabilities {
    ClientCapabilities {
        rendering,
        cols,
        rows,
        cell_px: cell_px.unwrap_or(CellPx { w: 8, h: 16 }),
        bracketed_paste: true,
        mouse: true,
        sixel_cols: None,
        sixel_rows: None,
    }
}

/// Detect terminal rendering capabilities.
///
/// Precedence:
/// 1. `PIXEL_AGENTS_TIER` env override (skips all I/O)
/// 2. Cached result from `~/.pixel-agents/capabilities-cache.json` (7-day TTL)
/// 3. Live probe: enable raw mode, send sequences, read replies with 150ms timeout
///
/// Returns `(caps, pre_app_bytes)`. `pre_app_bytes` is always empty: bytes typed
/// during the 150ms probe window are dropped by design (negligible at startup).
pub async fn detect() -> Result<(ClientCapabilities, Vec<u8>)> {
    // 1. Env override
    if let Ok(tier) = std::env::var("PIXEL_AGENTS_TIER") {
        let cap = parse_tier_override(&tier)?;
        return Ok((build_caps(terminal_cols(), terminal_rows(), cap, None), vec![]));
    }

    // 2. Cache hit
    if let Some(cached) = cache::read_cache() {
        let cell_px = cached.cell_px.map(|(w, h)| CellPx { w, h });
        return Ok((build_caps(terminal_cols(), terminal_rows(), cached.cap, cell_px), vec![]));
    }

    // 3. Live probe (only when stdout is an interactive terminal)
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        let cap = colorterm_fallback();
        return Ok((build_caps(terminal_cols(), terminal_rows(), cap, None), vec![]));
    }

    let (cap, cell_px) = live_probe().await?;
    let _ = cache::write_cache(&cap, cell_px.as_ref().map(|c| (c.w, c.h)));
    Ok((build_caps(terminal_cols(), terminal_rows(), cap, cell_px), vec![]))
}

async fn live_probe() -> Result<(RenderingCap, Option<CellPx>)> {
    // Enable raw mode so terminal replies arrive without line buffering
    let _raw = RawModeGuard::enable()?;

    let mut stdout = tokio::io::stdout();
    stdout.write_all(PROBE_DA1).await?;
    stdout.write_all(PROBE_KITTY).await?;
    stdout.write_all(PROBE_ITERM2).await?;
    stdout.write_all(PROBE_PIXEL_SIZE).await?;
    stdout.flush().await?;

    let raw_replies = read_replies_with_timeout().await;
    let probe = parse_replies(&raw_replies);

    let mux = in_multiplexer();

    let detected = if probe.has_kitty {
        if is_native_kitty() {
            RenderingCap::KittyK
        } else {
            RenderingCap::KittyO
        }
    } else if probe.has_iterm2 {
        RenderingCap::Iterm2
    } else if probe.has_sixel {
        RenderingCap::Sixel
    } else {
        colorterm_fallback()
    };

    // Multiplexers can't pass pixel graphics — demote anything above Sixel (T3)
    let cap = if mux { cap_at_most_sixel(detected) } else { detected };

    let cell_px = probe.cell_px.map(|(w, h)| CellPx { w, h });
    Ok((cap, cell_px))
}

/// Cap at T3 (Sixel): demote pixel-graphics tiers that can't pass through mux passthrough.
fn cap_at_most_sixel(cap: RenderingCap) -> RenderingCap {
    match cap {
        RenderingCap::KittyK | RenderingCap::KittyO | RenderingCap::Iterm2 => RenderingCap::Sixel,
        other => other,
    }
}

fn colorterm_fallback() -> RenderingCap {
    match std::env::var("COLORTERM").as_deref() {
        Ok("truecolor") | Ok("24bit") => RenderingCap::Truecolor,
        _ => RenderingCap::C256,
    }
}

async fn read_replies_with_timeout() -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut stdin = tokio::io::stdin();
    let mut replies = Vec::new();
    let mut buf = [0u8; 256];
    let deadline = tokio::time::Instant::now() + PROBE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, stdin.read(&mut buf)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(n)) => replies.extend_from_slice(&buf[..n]),
            Ok(Err(_)) => break,
        }
    }
    replies
}

fn parse_tier_override(s: &str) -> Result<RenderingCap> {
    let cap = match s.to_lowercase().as_str() {
        "t1-k" | "kittyk" | "kitty-k" => RenderingCap::KittyK,
        "t1-o" | "kittyo" | "kitty-o" => RenderingCap::KittyO,
        "t2" | "iterm2" => RenderingCap::Iterm2,
        "t3" | "sixel" => RenderingCap::Sixel,
        "t4" | "truecolor" => RenderingCap::Truecolor,
        "t5" | "256" | "c256" => RenderingCap::C256,
        "t6" | "16" | "c16" => RenderingCap::C16,
        "t7" | "braille" => RenderingCap::Braille,
        other => anyhow::bail!("unknown PIXEL_AGENTS_TIER value: {other}"),
    };
    Ok(cap)
}

fn terminal_cols() -> u16 {
    crossterm::terminal::size().map(|(c, _)| c).unwrap_or(220)
}

fn terminal_rows() -> u16 {
    crossterm::terminal::size().map(|(_, r)| r).unwrap_or(50)
}
