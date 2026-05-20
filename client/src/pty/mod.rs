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
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
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

/// Nominal cell pixel size reported to the terminal model. The exact values
/// don't matter for cell-tier rendering (we read glyphs, not pixels), but they
/// MUST be non-zero: the fork's image-placement math divides by the derived
/// `cell_pixel_width`/`height`, so a zero would panic on any inline image
/// escape (Kitty/iTerm2/Sixel) an agent emits.
const CELL_PX_W: u16 = 8;
const CELL_PX_H: u16 = 16;

/// Build a `TerminalSize` with non-zero pixel dimensions (see [`CELL_PX_W`]).
fn term_size(cols: u16, rows: u16) -> TerminalSize {
    let cols = cols.max(1);
    let rows = rows.max(1);
    TerminalSize {
        rows: rows as usize,
        cols: cols as usize,
        pixel_width: (cols * CELL_PX_W) as usize,
        pixel_height: (rows * CELL_PX_H) as usize,
        dpi: 96,
    }
}

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
        let term = Terminal::new(
            term_size(cols, rows),
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
        self.term.resize(term_size(cols, rows));
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

/// Encode a crossterm key press into the byte sequence a PTY expects, for
/// forwarding as `pty.input`. Returns `None` for keys we don't map (e.g. bare
/// modifier presses, F-keys) so the caller can ignore them.
///
/// Covers the interactive-MVP set: printable chars (with ALT → `ESC` prefix),
/// `Ctrl`+letter control codes, Enter/Tab/Backspace/Esc, arrows + nav keys.
/// Limitations (later slices): arrows are emitted in normal-cursor mode
/// (`ESC [ A`) regardless of the terminal's DECCKM application-cursor state,
/// and the Kitty keyboard protocol / full F-key set are not encoded.
pub fn encode_key(code: KeyCode, mods: KeyModifiers) -> Option<Vec<u8>> {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let mut out: Vec<u8> = Vec::new();
    match code {
        KeyCode::Char(c) => {
            if ctrl {
                // Map to the C0 control code; unknown combos are dropped.
                let b = match c.to_ascii_lowercase() {
                    'a'..='z' => (c.to_ascii_uppercase() as u8) & 0x1f,
                    ' ' | '@' => 0x00,
                    '[' => 0x1b,
                    '\\' => 0x1c,
                    ']' => 0x1d,
                    '^' => 0x1e,
                    '_' | '/' => 0x1f,
                    _ => return None,
                };
                if alt {
                    out.push(0x1b);
                }
                out.push(b);
            } else {
                if alt {
                    out.push(0x1b);
                }
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        KeyCode::Enter => out.push(b'\r'),
        KeyCode::Tab => out.push(b'\t'),
        KeyCode::BackTab => out.extend_from_slice(b"\x1b[Z"),
        KeyCode::Backspace => out.push(0x7f),
        KeyCode::Esc => out.push(0x1b),
        KeyCode::Up => out.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => out.extend_from_slice(b"\x1b[B"),
        KeyCode::Right => out.extend_from_slice(b"\x1b[C"),
        KeyCode::Left => out.extend_from_slice(b"\x1b[D"),
        KeyCode::Home => out.extend_from_slice(b"\x1b[H"),
        KeyCode::End => out.extend_from_slice(b"\x1b[F"),
        KeyCode::PageUp => out.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => out.extend_from_slice(b"\x1b[6~"),
        KeyCode::Delete => out.extend_from_slice(b"\x1b[3~"),
        KeyCode::Insert => out.extend_from_slice(b"\x1b[2~"),
        _ => return None,
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
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
    fn image_escapes_dont_garble_following_glyphs() {
        // Receipt for deferring the PtyByteTap *strip* half (plan §6 Day 4-5):
        // the wezterm parser already swallows image-protocol escapes, so glyphs
        // after them land normally. Only the *passthrough* half has value, and
        // that's blocked on image-tier live wiring (TODO), not Phase-4 sequence.
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        // Kitty APC: ESC _ G ... ESC \
        term.advance(b"\x1b_Gf=32,a=t,m=1;AAAA\x1b\\X");
        // iTerm2 OSC 1337: ESC ] 1337 ; ... BEL
        term.advance(b"\x1b]1337;File=name=Zg==:AAAA\x07Y");
        // Sixel DCS: ESC P ... ESC \
        term.advance(b"\x1bPq#0;2;100;0;0#0~~~\x1b\\Z");
        let mut buf = Buffer::empty(Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS));
        term.render_into(&mut buf, Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS));
        let row0: String = (0..3u16).map(|x| buf[(x, 0)].symbol()).collect();
        assert_eq!(row0, "XYZ", "image escapes must be parsed-and-ignored, not garble glyphs");
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

    #[test]
    fn encode_plain_char() {
        assert_eq!(encode_key(KeyCode::Char('a'), KeyModifiers::NONE), Some(b"a".to_vec()));
    }

    #[test]
    fn encode_ctrl_c_is_etx() {
        assert_eq!(
            encode_key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(vec![0x03])
        );
    }

    #[test]
    fn encode_enter_is_carriage_return() {
        assert_eq!(encode_key(KeyCode::Enter, KeyModifiers::NONE), Some(b"\r".to_vec()));
    }

    #[test]
    fn encode_backspace_is_del() {
        assert_eq!(encode_key(KeyCode::Backspace, KeyModifiers::NONE), Some(vec![0x7f]));
    }

    #[test]
    fn encode_arrow_up_is_csi() {
        assert_eq!(encode_key(KeyCode::Up, KeyModifiers::NONE), Some(b"\x1b[A".to_vec()));
    }

    #[test]
    fn encode_alt_char_prefixes_escape() {
        assert_eq!(
            encode_key(KeyCode::Char('x'), KeyModifiers::ALT),
            Some(b"\x1bx".to_vec())
        );
    }

    #[test]
    fn encode_unmapped_key_is_none() {
        assert_eq!(encode_key(KeyCode::F(5), KeyModifiers::NONE), None);
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
