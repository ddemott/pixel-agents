#![allow(dead_code)]
// Capability probe sequences and reply parsers.
// All I/O is supplied by the caller so these functions are sync and testable.

use vte::{Params, Parser, Perform};

// ── Probe byte sequences ──────────────────────────────────────────────────────

/// DA1 — "Send Device Attributes" (primary)
pub const PROBE_DA1: &[u8] = b"\x1b[c";

/// Kitty graphics: load 1-pixel PNG, action=query, delete-after-display
pub const PROBE_KITTY: &[u8] =
    b"\x1b_Gi=99,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\";

/// iTerm2 — request terminal identification via OSC 1337
pub const PROBE_ITERM2: &[u8] = b"\x1b]1337;ReportCellSize\x07";

/// CSI 14 t — request terminal window size in pixels
pub const PROBE_PIXEL_SIZE: &[u8] = b"\x1b[14t";

// ── Reply parsers ─────────────────────────────────────────────────────────────

/// Parsed capabilities from probing the terminal.
#[derive(Debug, Default, Clone)]
pub struct ProbeResult {
    pub has_sixel: bool,
    pub has_kitty: bool,
    pub has_iterm2: bool,
    /// Cell size in pixels: (width, height)
    pub cell_px: Option<(u16, u16)>,
}

// vte 0.15 silently discards APC (ESC _) body bytes (SosPmApcString → anywhere()).
// Kitty replies (APC) must be parsed by scanning raw bytes directly.
// vte handles CSI (DA1, CSI 14t) and OSC (iTerm2 1337) correctly.

struct ReplyParser {
    result: ProbeResult,
}

impl ReplyParser {
    fn new() -> Self {
        Self { result: ProbeResult::default() }
    }
}

impl Perform for ReplyParser {
    // DA1 reply: ESC [ ? <params> c
    // Sixel = param 4 in list
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        if action == 'c' && intermediates == b"?" {
            for param in params.iter() {
                for &p in param {
                    if p == 4 {
                        self.result.has_sixel = true;
                    }
                }
            }
        }
        // CSI 14 t reply: ESC [ 4 ; <h> ; <w> t
        if action == 't' {
            let mut ps = params.iter().flat_map(|p| p.iter().copied());
            let first = ps.next().unwrap_or(0);
            if first == 4 {
                let h = ps.next().unwrap_or(0);
                let w = ps.next().unwrap_or(0);
                if w > 0 && h > 0 {
                    self.result.cell_px = Some((w as u16, h as u16));
                }
            }
        }
    }

    // iTerm2 reply arrives as OSC 1337 ; ... BEL/ST
    // body looks like "CellSize=<h>x<w>"
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.len() >= 2 && params[0] == b"1337" {
            let body = params[1];
            if let Some(rest) = body.strip_prefix(b"CellSize=") {
                self.result.has_iterm2 = true;
                if let Some(x) = rest.iter().position(|&b| b == b'x') {
                    let h_str = std::str::from_utf8(&rest[..x]).unwrap_or("");
                    let w_str = std::str::from_utf8(&rest[x + 1..]).unwrap_or("");
                    if let (Ok(h), Ok(w)) = (h_str.parse::<f32>(), w_str.parse::<f32>()) {
                        let h = h.round() as u16;
                        let w = w.round() as u16;
                        if w > 0 && h > 0 {
                            self.result.cell_px = Some((w, h));
                        }
                    }
                }
            }
            if body.windows(6).any(|w| w == b"iTerm2") {
                self.result.has_iterm2 = true;
            }
        }
    }
}

/// Scan raw bytes for Kitty APC reply: `ESC _ G i=<id>;OK ESC \`
/// vte 0.15 discards APC body, so we must scan manually.
fn scan_kitty_ok(bytes: &[u8]) -> bool {
    // Look for ESC _ G ... ;OK ... ESC \
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == 0x1b && bytes[i + 1] == b'_' {
            // Find ST (ESC \)
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == 0x1b && bytes[j + 1] == b'\\' {
                    let body = &bytes[start..j];
                    if body.first() == Some(&b'G') && body.windows(2).any(|w| w == b"OK") {
                        return true;
                    }
                    i = j + 2;
                    break;
                }
                j += 1;
            }
            if j + 1 >= bytes.len() {
                break;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// Parse raw terminal reply bytes into `ProbeResult`.
/// Feed all bytes collected during the probe timeout here.
pub fn parse_replies(bytes: &[u8]) -> ProbeResult {
    let mut parser = Parser::new();
    let mut performer = ReplyParser::new();
    parser.advance(&mut performer, bytes);
    let mut result = performer.result;
    // vte 0.15 discards APC body; scan raw bytes for Kitty reply
    if scan_kitty_ok(bytes) {
        result.has_kitty = true;
    }
    result
}

// ── Env-based Kitty variant heuristic ────────────────────────────────────────

/// Returns true if env suggests the terminal natively supports Kitty (T1-K),
/// i.e. `KITTY_WINDOW_ID` is set, `TERM_PROGRAM=ghostty`, or `TERM=xterm-kitty`.
pub fn is_native_kitty() -> bool {
    std::env::var("KITTY_WINDOW_ID").is_ok()
        || std::env::var("TERM_PROGRAM").as_deref() == Ok("ghostty")
        || std::env::var("TERM").as_deref() == Ok("xterm-kitty")
}

/// Returns true if running inside a terminal multiplexer (tmux or zellij).
pub fn in_multiplexer() -> bool {
    std::env::var("TMUX").is_ok() || std::env::var("ZELLIJ").is_ok()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn da1_sixel_detected() {
        // ESC[?64;1;4;6;9;15;22;28;32;42c — parameter 4 present
        let reply = b"\x1b[?64;1;4;6;9;15;22;28;32;42c";
        let r = parse_replies(reply);
        assert!(r.has_sixel);
    }

    #[test]
    fn da1_no_sixel() {
        let reply = b"\x1b[?64;1;6;9;15c";
        let r = parse_replies(reply);
        assert!(!r.has_sixel);
    }

    #[test]
    fn kitty_ok() {
        // APC: ESC _ G i=99;OK ESC \
        let reply = b"\x1b_Gi=99;OK\x1b\\";
        let r = parse_replies(reply);
        assert!(r.has_kitty);
    }

    #[test]
    fn kitty_error_not_ok() {
        let reply = b"\x1b_Gi=99;EINVAL:unsupported pixel format\x1b\\";
        let r = parse_replies(reply);
        assert!(!r.has_kitty);
    }

    #[test]
    fn csi14t_cell_size() {
        // ESC[4;20;10t — height=20, width=10
        let reply = b"\x1b[4;20;10t";
        let r = parse_replies(reply);
        assert_eq!(r.cell_px, Some((10, 20)));
    }

    #[test]
    fn iterm2_cell_size() {
        // OSC 1337 ; CellSize=20.0x10.0 ST
        let reply = b"\x1b]1337;CellSize=20.0x10.0\x07";
        let r = parse_replies(reply);
        assert!(r.has_iterm2);
        assert_eq!(r.cell_px, Some((10, 20)));
    }

    #[test]
    fn mixed_replies_all_detected() {
        let mut data = Vec::new();
        data.extend_from_slice(b"\x1b[?4c");      // sixel DA1 (minimal)
        data.extend_from_slice(b"\x1b_Gi=99;OK\x1b\\"); // kitty
        data.extend_from_slice(b"\x1b[4;24;12t"); // cell size
        let r = parse_replies(&data);
        assert!(r.has_sixel);
        assert!(r.has_kitty);
        assert_eq!(r.cell_px, Some((12, 24)));
    }
}
