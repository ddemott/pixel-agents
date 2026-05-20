//! PTY hosting — client side (Phase 4).
//!
//! The daemon spawns each agent's PTY and streams its raw output to every
//! authed client over the binary multiplex (frame tag `0x01`, see
//! `daemon/src/agents/broadcastSink.ts::broadcastPty`). This module turns that
//! byte stream into a renderable terminal grid: one [`PtyTerminal`] per agent,
//! fed via [`PtyTerminal::advance`], drawn into a Ratatui buffer by
//! [`PtyTerminal::render_into`] when that agent is focused.
//!
//! The terminal model is the published Tattoy fork of `wezterm-term`
//! (`tattoy-wezterm-term`; upstream was never on crates.io — see
//! `docs/tui-implementation-plan.md` §6). Bytes are parsed by
//! `Terminal::advance_bytes`, which mutates the grid; the visible rows are read
//! back through `screen().visible_lines()`.
//!
//! Scope (slice 1): ingest + render the focused agent's screen. Deferred to
//! later Phase-4 slices: input forwarding (`pty.input`), resize follow
//! (`pty.resize`), the Kitty/iTerm2/Sixel-aware [`PtyByteTap`], scrollback
//! display, and answerback routing (the `Terminal` writer below is a sink, so
//! replies to DA/cursor queries are currently discarded rather than sent back
//! up as `pty.input`).

use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
// `ColorAttribute` + `ColorPalette` both live in the term crate's `color`
// module (it re-exports `wezterm_cell::color::ColorAttribute`).
use tattoy_wezterm_term::color::{ColorAttribute, ColorPalette};
use tattoy_wezterm_term::{Terminal, TerminalConfiguration, TerminalSize};

/// Default PTY grid the daemon spawns agents at (node-pty default), used until
/// resize-follow lands. Matches `TerminalSize::default()`.
pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

/// Minimal terminal config: only `color_palette` is required (everything else
/// on the trait has a default). We hand back the standard xterm palette; live
/// OSC-4 palette changes are not tracked yet (slice 1).
#[derive(Debug)]
struct PtyConfig;

impl TerminalConfiguration for PtyConfig {
    fn color_palette(&self) -> ColorPalette {
        ColorPalette::default()
    }
}

/// One agent's headless terminal: parses PTY output into a cell grid we can draw.
pub struct PtyTerminal {
    term: Terminal,
    /// Palette used to resolve cell `ColorAttribute`s to RGB at render time.
    palette: ColorPalette,
}

impl PtyTerminal {
    /// New terminal sized `cols × rows`. The writer is a sink: answerback bytes
    /// (DA/cursor-position replies) are discarded for now — routing them back as
    /// `pty.input` is a later slice.
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = TerminalSize {
            rows: rows.max(1) as usize,
            cols: cols.max(1) as usize,
            ..TerminalSize::default()
        };
        let term = Terminal::new(
            size,
            Arc::new(PtyConfig),
            "PixelAgents",
            env!("CARGO_PKG_VERSION"),
            Box::new(std::io::sink()),
        );
        Self { term, palette: ColorPalette::default() }
    }

    /// Feed a chunk of raw PTY output. Chunks need not align to escape
    /// sequences — the parser carries state across calls.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.term.advance_bytes(bytes);
    }

    /// Resize the grid (used once resize-follow lands; harmless before then).
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(TerminalSize {
            rows: rows.max(1) as usize,
            cols: cols.max(1) as usize,
            ..TerminalSize::default()
        });
    }

    /// Current grid size as `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        let s = self.term.get_size();
        (s.cols as u16, s.rows as u16)
    }

    /// Paint the visible screen into `area`. Clears `area` to the terminal's
    /// default background first (so blank cells don't show stale content from a
    /// prior view), then writes each occupied cell with its resolved fg/bg, and
    /// finally inverts the cursor cell when visible.
    pub fn render_into(&self, buf: &mut Buffer, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Clear to default bg.
        let (dbr, dbg, dbb, _) = self.palette.resolve_bg(ColorAttribute::Default).to_srgb_u8();
        let default_bg = Color::Rgb(dbr, dbg, dbb);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let cell = &mut buf[(x, y)];
                cell.set_symbol(" ");
                cell.set_bg(default_bg);
                cell.set_fg(default_bg);
            }
        }

        // `Screen::visible_lines` is test-only in this crate, so read the
        // visible window via the phys-range API: map visible rows `0..rows` to
        // physical row indices, then copy those lines.
        let rows = self.term.get_size().rows as i64;
        let screen = self.term.screen();
        let phys = screen.phys_range(&(0..rows));
        let lines = screen.lines_in_phys_range(phys);
        for (y, line) in lines.iter().enumerate() {
            if y as u16 >= area.height {
                break;
            }
            let row = area.y + y as u16;
            for cell_ref in line.visible_cells() {
                let x = cell_ref.cell_index();
                if x as u16 >= area.width {
                    continue;
                }
                let attrs = cell_ref.attrs();
                let (fr, fg, fb, _) = self.palette.resolve_fg(attrs.foreground()).to_srgb_u8();
                let (br, bg, bb, _) = self.palette.resolve_bg(attrs.background()).to_srgb_u8();
                let cell = &mut buf[(area.x + x as u16, row)];
                let s = cell_ref.str();
                if !s.is_empty() {
                    cell.set_symbol(s);
                }
                cell.set_fg(Color::Rgb(fr, fg, fb));
                cell.set_bg(Color::Rgb(br, bg, bb));
            }
        }

        // Cursor: invert the fg/bg of its cell when on-screen.
        let cursor = self.term.cursor_pos();
        if cursor.y >= 0 && (cursor.y as u16) < area.height && (cursor.x as u16) < area.width {
            let cell = &mut buf[(area.x + cursor.x as u16, area.y + cursor.y as u16)];
            let (fg, bg) = (cell.fg, cell.bg);
            cell.set_fg(bg);
            cell.set_bg(fg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_plain_text_into_grid() {
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        term.advance(b"hi");
        let mut buf = Buffer::empty(Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS));
        term.render_into(&mut buf, Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS));
        assert_eq!(buf[(0, 0)].symbol(), "h");
        assert_eq!(buf[(1, 0)].symbol(), "i");
    }

    #[test]
    fn newline_advances_row() {
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        // CRLF since the PTY is in raw mode (no ONLCR translation on our side).
        term.advance(b"a\r\nb");
        let mut buf = Buffer::empty(Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS));
        term.render_into(&mut buf, Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS));
        assert_eq!(buf[(0, 0)].symbol(), "a");
        assert_eq!(buf[(0, 1)].symbol(), "b");
    }

    #[test]
    fn split_escape_across_advances_is_parsed() {
        // SGR red fg "\x1b[31m" delivered in two chunks, then a glyph.
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        term.advance(b"\x1b[3");
        term.advance(b"1mX");
        let mut buf = Buffer::empty(Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS));
        term.render_into(&mut buf, Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS));
        assert_eq!(buf[(0, 0)].symbol(), "X");
        // Red resolves to a non-default fg; just assert it parsed as a glyph at 0,0.
    }

    #[test]
    fn render_clips_to_smaller_area() {
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        term.advance(b"abcdef");
        // 3-wide area: only a,b,c land; no panic past the edge.
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 2));
        term.render_into(&mut buf, Rect::new(0, 0, 3, 2));
        assert_eq!(buf[(0, 0)].symbol(), "a");
        assert_eq!(buf[(2, 0)].symbol(), "c");
    }

    /// Compile-time guard: catches a renamed fork API at `cargo test` time.
    #[test]
    fn dependency_resolves() {
        fn _api_smoke(term: &mut Terminal) {
            term.advance_bytes(b"");
        }
        let _ = _api_smoke;
    }
}
