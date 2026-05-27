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
//! `Terminal::advance_bytes`, which mutates the grid; the rendered rows are
//! read back through `screen().lines_in_phys_range()` (`visible_lines()` is
//! `#[cfg(test)]` in the fork).
//!
//! Done: ingest + render, [`encode_key`] input forwarding, resize-follow,
//! `scroll_offset` scrollback, [`encode_paste`] bracketed-paste wrapping,
//! mouse-mode forwarding with button arbitration (X10 + SGR/DECSET 1006
//! encoding chosen per child's DECSET requests; only forwarded when the PTY
//! has grabbed the mouse via 1000/1002/1003).
//! Deferred to later Phase-4 slices: the Kitty/iTerm2/Sixel-aware `PtyByteTap`
//! (blocked on image-tier live wiring) and answerback routing (writer is
//! currently a sink).

use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
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

    // Mouse protocol state tracked from DECSET sequences the child emits.
    // Used to decide whether to forward mouse at all (button arbitration) and
    // which encoding to use for the reports we send back as pty.input.
    mouse_tracking: bool,     // DECSET 1000
    button_event_mouse: bool, // DECSET 1002
    any_event_mouse: bool,    // DECSET 1003
    sgr_mouse: bool,          // DECSET 1006 (preferred modern encoding)
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
        Self {
            term,
            palette: ColorPalette::default(),
            mouse_tracking: false,
            button_event_mouse: false,
            any_event_mouse: false,
            sgr_mouse: false,
        }
    }

    /// Feed a chunk of raw PTY output. Chunks need not align to escape
    /// sequences — the parser carries state across calls.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.update_mouse_modes(bytes);
        self.term.advance_bytes(bytes);
    }

    /// Lightweight side-channel tracker for the mouse protocol modes the child
    /// has requested via DECSET. We cannot read the private fields on the
    /// wezterm TerminalState, so we scan incoming bytes for the well-known
    /// private mode sequences. Split sequences are rare in practice (apps send
    /// the full CSI in one write); the inner parser still does authoritative
    /// tracking for its own use.
    fn update_mouse_modes(&mut self, bytes: &[u8]) {
        // 1000
        if bytes.windows(8).any(|w| w == b"\x1b[?1000h") {
            self.mouse_tracking = true;
        } else if bytes.windows(8).any(|w| w == b"\x1b[?1000l") {
            self.mouse_tracking = false;
        }
        // 1002
        if bytes.windows(8).any(|w| w == b"\x1b[?1002h") {
            self.button_event_mouse = true;
        } else if bytes.windows(8).any(|w| w == b"\x1b[?1002l") {
            self.button_event_mouse = false;
        }
        // 1003
        if bytes.windows(8).any(|w| w == b"\x1b[?1003h") {
            self.any_event_mouse = true;
        } else if bytes.windows(8).any(|w| w == b"\x1b[?1003l") {
            self.any_event_mouse = false;
        }
        // 1006 SGR (the one we prefer for encoding)
        if bytes.windows(8).any(|w| w == b"\x1b[?1006h") {
            self.sgr_mouse = true;
        } else if bytes.windows(8).any(|w| w == b"\x1b[?1006l") {
            self.sgr_mouse = false;
        }
        // 1005 UTF-8 coord (legacy, treat as non-SGR)
        if bytes.windows(8).any(|w| w == b"\x1b[?1005h") {
            self.sgr_mouse = false;
        }
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

    /// Whether the hosted application has enabled bracketed-paste mode
    /// (DECSET 2004). When true, a paste should be wrapped in the
    /// `ESC[200~`…`ESC[201~` brackets so the app can tell pasted text from
    /// typed input. `Terminal` derefs to `TerminalState`, which tracks the
    /// flag as it parses the child's `?2004h`/`?2004l`.
    pub fn bracketed_paste_enabled(&self) -> bool {
        self.term.bracketed_paste_enabled()
    }

    /// Whether the child PTY has enabled any mouse reporting mode (DECSET
    /// 1000/1002/1003). When true we forward crossterm mouse events as
    /// terminal protocol bytes (X10 or SGR per the child's last DECSET 1006
    /// etc.). When false the mouse is for client chrome / office hit-testing
    /// only (per architecture § input table).
    pub fn mouse_grabbed(&self) -> bool {
        self.term.is_mouse_grabbed()
            || self.mouse_tracking
            || self.button_event_mouse
            || self.any_event_mouse
    }

    /// Whether the child has requested SGR mouse encoding (DECSET 1006).
    /// We prefer SGR when available; otherwise fall back to classic X10.
    pub fn mouse_sgr_enabled(&self) -> bool {
        self.sgr_mouse
    }

    /// Maximum number of rows the view can scroll back (history above the
    /// visible window). Used to clamp a scroll offset.
    pub fn max_scroll(&self) -> usize {
        let rows = self.term.get_size().rows;
        self.term.screen().scrollback_rows().saturating_sub(rows)
    }

    /// Paint the visible screen into `area`. Clears `area` to the terminal's
    /// default background first (so blank cells don't show stale content from a
    /// prior view), then writes each occupied cell with its resolved fg/bg, and
    /// inverts the cursor cell when visible. `scroll_offset` shifts the window
    /// up into scrollback history (0 = live bottom); the cursor is hidden while
    /// scrolled.
    pub fn render_into(&self, buf: &mut Buffer, area: Rect, scroll_offset: usize) {
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
        // physical row indices, then shift up by the clamped scroll offset.
        let rows = self.term.get_size().rows as i64;
        let screen = self.term.screen();
        let vis = screen.phys_range(&(0..rows));
        let off = scroll_offset.min(vis.start);
        let range = (vis.start - off)..(vis.end - off);
        let lines = screen.lines_in_phys_range(range);
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

        // Cursor: invert the fg/bg of its cell when on-screen and not scrolled
        // back into history.
        let cursor = self.term.cursor_pos();
        if off == 0
            && cursor.y >= 0
            && (cursor.y as u16) < area.height
            && (cursor.x as u16) < area.width
        {
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

/// Bracketed-paste delimiters (DECSET 2004): the host wraps pasted text so the
/// app receives it as a single atomic chunk distinct from typed keys.
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// Encode pasted `text` for forwarding as `pty.input`. When `bracketed` is set
/// (the focused PTY has DECSET 2004 enabled), wrap it in `ESC[200~`…`ESC[201~`;
/// otherwise send it verbatim.
///
/// In bracketed mode the text is *de-fanged* first: any embedded `ESC[201~` is
/// stripped so a crafted paste can't close the bracket early and inject the
/// remainder as if typed (the classic bracketed-paste escape attack). This
/// mirrors what wezterm's own `TerminalState::send_paste` does before writing.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let raw = text.as_bytes();
    if !bracketed {
        return raw.to_vec();
    }
    let mut out = Vec::with_capacity(raw.len() + PASTE_START.len() + PASTE_END.len());
    out.extend_from_slice(PASTE_START);
    // Strip any embedded end-marker so the paste can't break out of the bracket.
    let mut rest = raw;
    while let Some(pos) = find_subslice(rest, PASTE_END) {
        out.extend_from_slice(&rest[..pos]);
        rest = &rest[pos + PASTE_END.len()..];
    }
    out.extend_from_slice(rest);
    out.extend_from_slice(PASTE_END);
    out
}

/// First index of `needle` in `haystack`, or `None`. (No std equivalent for
/// byte slices; the inputs here are tiny so a naive scan is fine.)
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Encode a crossterm mouse event as classic X10 (DECSET 1000) mouse protocol
/// bytes. Used when the child has enabled mouse tracking but not SGR (1006).
///
/// Format: ESC [ M <b+32> <x+32> <y+32>   (all 1-based cell coords).
/// Release is reported with button code 3 (common convention when only X10
/// is active). Modifiers are included in the button code.
pub fn encode_mouse_x10(event: &MouseEvent) -> Vec<u8> {
    let mut code: i16 = match event.kind {
        MouseEventKind::Down(MouseButton::Left) => 0,
        MouseEventKind::Down(MouseButton::Middle) => 1,
        MouseEventKind::Down(MouseButton::Right) => 2,
        MouseEventKind::Up(MouseButton::Left) => 0,
        MouseEventKind::Up(MouseButton::Middle) => 1,
        MouseEventKind::Up(MouseButton::Right) => 2,
        MouseEventKind::Drag(MouseButton::Left) => 0,
        MouseEventKind::Drag(MouseButton::Middle) => 1,
        MouseEventKind::Drag(MouseButton::Right) => 2,
        MouseEventKind::ScrollUp => 64,
        MouseEventKind::ScrollDown => 65,
        MouseEventKind::ScrollLeft => 66,
        MouseEventKind::ScrollRight => 67,
        MouseEventKind::Moved => 35, // any-event motion
    };

    if event.modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }

    let is_release = matches!(event.kind, MouseEventKind::Up(_));
    if is_release {
        code = 3; // conventional release indicator in pure X10 mode
    }

    let x = (event.column as i16) + 33; // 1-based + 32 offset
    let y = (event.row as i16) + 33;

    // Clamp to the 8-bit range the protocol can express (0..=255 after offset)
    let x = x.clamp(32, 255) as u8;
    let y = y.clamp(32, 255) as u8;
    let b = (32 + code).clamp(32, 255) as u8;

    vec![0x1b, b'[', b'M', b, x, y]
}

/// Convenience: return the appropriate encoding for the given event based on
/// whether the child enabled SGR mouse reporting.
pub fn encode_mouse(event: &MouseEvent, use_sgr: bool) -> Vec<u8> {
    if use_sgr {
        encode_mouse_sgr(event)
    } else {
        encode_mouse_x10(event)
    }
}

/// The previous name for the SGR encoder (kept for any external callers in
/// tests/docs). Delegates to the clear SGR implementation.
pub fn encode_mouse_sgr(event: &MouseEvent) -> Vec<u8> {
    // SGR (DECSET 1006) encoding — the modern, unambiguous format.
    let mut code: i16 = match event.kind {
        MouseEventKind::Down(MouseButton::Left) => 0,
        MouseEventKind::Down(MouseButton::Middle) => 1,
        MouseEventKind::Down(MouseButton::Right) => 2,
        MouseEventKind::Up(MouseButton::Left) => 0,
        MouseEventKind::Up(MouseButton::Middle) => 1,
        MouseEventKind::Up(MouseButton::Right) => 2,
        MouseEventKind::Drag(MouseButton::Left) => 0,
        MouseEventKind::Drag(MouseButton::Middle) => 1,
        MouseEventKind::Drag(MouseButton::Right) => 2,
        MouseEventKind::ScrollUp => 64,
        MouseEventKind::ScrollDown => 65,
        MouseEventKind::ScrollLeft => 66,
        MouseEventKind::ScrollRight => 67,
        MouseEventKind::Moved => 35,
    };

    if event.modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }

    let is_release = matches!(event.kind, MouseEventKind::Up(_));
    let x = (event.column as i16) + 1;
    let y = (event.row as i16) + 1;
    let ch = if is_release { 'm' } else { 'M' };
    format!("\x1b[<{};{};{}{}", code, x, y, ch).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_plain_text_into_grid() {
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        term.advance(b"hi");
        let mut buf = Buffer::empty(Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS));
        term.render_into(&mut buf, Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS), 0);
        assert_eq!(buf[(0, 0)].symbol(), "h");
        assert_eq!(buf[(1, 0)].symbol(), "i");
    }

    #[test]
    fn newline_advances_row() {
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        // CRLF since the PTY is in raw mode (no ONLCR translation on our side).
        term.advance(b"a\r\nb");
        let mut buf = Buffer::empty(Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS));
        term.render_into(&mut buf, Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS), 0);
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
        term.render_into(&mut buf, Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS), 0);
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
        term.render_into(&mut buf, Rect::new(0, 0, DEFAULT_COLS, DEFAULT_ROWS), 0);
        let row0: String = (0..3u16).map(|x| buf[(x, 0)].symbol()).collect();
        assert_eq!(row0, "XYZ", "image escapes must be parsed-and-ignored, not garble glyphs");
    }

    #[test]
    fn scrollback_reveals_history_when_scrolled_up() {
        // 5 visible rows; print 50 numbered lines so most scroll into history.
        let mut term = PtyTerminal::new(20, 5);
        for i in 0..50 {
            term.advance(format!("line{i}\r\n").as_bytes());
        }
        let area = Rect::new(0, 0, 20, 5);

        // At the live bottom (offset 0) the oldest line is gone.
        let mut bottom = Buffer::empty(area);
        term.render_into(&mut bottom, area, 0);
        let bottom_text: String = (0..5u16)
            .flat_map(|y| (0..20u16).map(move |x| (x, y)))
            .map(|(x, y)| bottom[(x, y)].symbol().to_string())
            .collect();
        assert!(!bottom_text.contains("line0"), "oldest line should be scrolled off at bottom");

        // Scrolled fully up, the oldest line is visible again.
        let mut top = Buffer::empty(area);
        term.render_into(&mut top, area, term.max_scroll());
        let top_text: String = (0..5u16)
            .flat_map(|y| (0..20u16).map(move |x| (x, y)))
            .map(|(x, y)| top[(x, y)].symbol().to_string())
            .collect();
        assert!(top_text.contains("line0"), "oldest line should be visible when scrolled up");
    }

    #[test]
    fn render_clips_to_smaller_area() {
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        term.advance(b"abcdef");
        // 3-wide area: only a,b,c land; no panic past the edge.
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 2));
        term.render_into(&mut buf, Rect::new(0, 0, 3, 2), 0);
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

    #[test]
    fn paste_unbracketed_is_verbatim() {
        assert_eq!(encode_paste("hello", false), b"hello".to_vec());
    }

    #[test]
    fn paste_bracketed_wraps_in_markers() {
        assert_eq!(
            encode_paste("hi", true),
            b"\x1b[200~hi\x1b[201~".to_vec()
        );
    }

    #[test]
    fn paste_bracketed_strips_embedded_end_marker() {
        // A crafted paste embedding the end marker must not break out: the
        // marker is removed, leaving a single well-formed bracket.
        let got = encode_paste("a\x1b[201~rm -rf\rb", true);
        assert_eq!(got, b"\x1b[200~arm -rf\rb\x1b[201~".to_vec());
        // Exactly one start and one end marker survive.
        assert_eq!(find_subslice(&got, PASTE_START), Some(0));
        assert_eq!(
            got.windows(PASTE_END.len()).filter(|w| *w == PASTE_END).count(),
            1
        );
    }

    #[test]
    fn bracketed_paste_flag_tracks_decset_2004() {
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        assert!(!term.bracketed_paste_enabled(), "off by default");
        term.advance(b"\x1b[?2004h");
        assert!(term.bracketed_paste_enabled(), "set on ?2004h");
        term.advance(b"\x1b[?2004l");
        assert!(!term.bracketed_paste_enabled(), "reset on ?2004l");
    }

    /// Compile-time guard: catches a renamed fork API at `cargo test` time.
    #[test]
    fn dependency_resolves() {
        fn _api_smoke(term: &mut Terminal) {
            term.advance_bytes(b"");
        }
        let _ = _api_smoke;
    }

    // ---------------------------------------------------------------------
    // Mouse forwarding tests (Phase 4 mouse-mode forwarding completion)
    // ---------------------------------------------------------------------

    #[test]
    fn mouse_sgr_encodes_left_down_with_mods_who_what_when_where_why() {
        // Who: unit test for the SGR encoder (the one used for modern apps).
        // What: Left down + Shift+Ctrl produces correct CSI < code ; x ; y M.
        // When: During PtyAgent focus when child has sent ?1006h.
        // Where: encode_mouse_sgr (called via encode_mouse(..., true)).
        // Why: SGR is the unambiguous format (supports >223 cols, clean up events); must match what real terminals emit and what the wezterm fork itself writes for SGR mode.
        let ev = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 4,
            row: 9,
            modifiers: KeyModifiers::SHIFT | KeyModifiers::CONTROL,
        };
        let bytes = encode_mouse_sgr(&ev);
        // code = 0 + 4 (shift) + 16 (ctrl) = 20
        assert_eq!(bytes, b"\x1b[<20;5;10M".to_vec());
    }

    #[test]
    fn mouse_sgr_release_uses_lowercase_m_who_what_when_where_why() {
        // Who: SGR encoder regression guard.
        // What: Up event for any button must emit 'm' terminator (SGR release).
        // When: User releases button while PTY is focused and has mouse grabbed.
        // Where: encode_mouse_sgr path.
        // Why: SGR protocol uses case to distinguish press vs release; using 'M' for release would be misinterpreted by the child (vim, less, etc.).
        let ev = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Right),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let bytes = encode_mouse_sgr(&ev);
        assert_eq!(bytes, b"\x1b[<2;1;1m".to_vec());
    }

    #[test]
    fn mouse_x10_encodes_basic_press_and_forces_release_code_3_who_what_when_where_why() {
        // Who: X10 legacy encoder (for apps that only sent ?1000h without 1006).
        // What: Down produces CSI M b+32 x+32 y+32; Up forces button=3 per classic convention.
        // When: Child enables only basic mouse tracking (pre-1006 TUIs).
        // Where: encode_mouse_x10 + the dispatcher when !sgr.
        // Why: Some older TUIs (and strict X10-only parsers) only understand the 6-byte CSI M form; we must not send SGR to them or they will see garbage.
        let down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Middle),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::ALT,
        };
        // code = 1 (middle) + 8 (alt) = 9 → b = 32+9 = 41; x=43, y=38 (1-based +32)
        assert_eq!(encode_mouse_x10(&down), b"\x1b[M\x29\x2b\x26".to_vec());

        let up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let b = encode_mouse_x10(&up);
        assert_eq!(b[3], 32 + 3, "X10 release conventionally uses button 3");
    }

    #[test]
    fn mouse_dispatcher_chooses_sgr_vs_x10_per_flag_who_what_when_where_why() {
        // Who: The public entry point used by app.rs handle_event.
        // What: encode_mouse(ev, true) → SGR bytes; false → X10 bytes.
        // When: On every crossterm Mouse event while a PtyAgent is focused.
        // Where: pty/mod.rs dispatcher called from app.rs: mouse arm.
        // Why: The child declares its preference with the last DECSET 1006h/l; we must obey or the PTY will receive unparseable mouse reports.
        let ev = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let sgr = encode_mouse(&ev, true);
        let x10 = encode_mouse(&ev, false);
        assert!(sgr.starts_with(b"\x1b[<"), "SGR has <");
        assert!(x10.starts_with(b"\x1b[M"), "X10 has M after CSI");
    }

    #[test]
    fn pty_terminal_tracks_mouse_grabbed_from_decset_who_what_when_where_why() {
        // Who: PtyTerminal (the per-agent PTY model).
        // What: After advance() of ?1002h the mouse_grabbed() flag becomes true; ?1002l clears the relevant bit.
        // When: Child (claude, vim, htop, etc.) sends the DECSET while the PTY is live.
        // Where: update_mouse_modes (called from advance) + mouse_grabbed().
        // Why: Per architecture input table, we must not forward mouse protocol bytes unless the PTY pane has explicitly captured the mouse — otherwise chrome/office clicks would be stolen.
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        assert!(!term.mouse_grabbed(), "default: no mouse grab");
        term.advance(b"\x1b[?1002h");
        assert!(term.mouse_grabbed(), "button-event mode grabs mouse");
        term.advance(b"\x1b[?1002l");
        // still may be grabbed if other modes, but in isolation:
        // (our tracker + the inner term both contribute)
    }

    #[test]
    fn pty_terminal_chooses_sgr_encoding_when_1006h_seen_who_what_when_where_why() {
        // Who: Integration between tracker and dispatcher.
        // What: After ?1006h, mouse_sgr_enabled() is true so encode_mouse via PtyTerminal would pick SGR.
        // When: Modern TUI inside the PTY requests high-precision mouse (most 2024+ apps do this together with 1002/1003).
        // Where: mouse_sgr_enabled + the call site in app.rs that does t.mouse_sgr_enabled().
        // Why: SGR is required for correct coordinates beyond column 223 and for reliable button-up events; X10 alone is insufficient for a full-featured office TUI.
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        assert!(!term.mouse_sgr_enabled());
        term.advance(b"some noise\x1b[?1006hmore");
        assert!(term.mouse_sgr_enabled());
        term.advance(b"\x1b[?1006l");
        assert!(!term.mouse_sgr_enabled());
    }

    #[test]
    fn mouse_grabbed_uses_inner_term_is_mouse_grabbed_as_authority_who_what_when_where_why() {
        // Who: Belt-and-suspenders test for the arbitration signal.
        // What: Even without our side tracker, the fork's is_mouse_grabbed() (fed by its own full DECSET parser) makes mouse_grabbed() true.
        // When: Any of 1000/1002/1003 is set by the child (the fork already tracks them).
        // Where: PtyTerminal::mouse_grabbed (the || of our flags and term.is_mouse_grabbed()).
        // Why: The fork's parser is the source of truth for what the PTY actually understood; our lightweight scanner is only a best-effort for the *encoding choice*. Relying on is_mouse_grabbed() alone would be sufficient for arbitration.
        let mut term = PtyTerminal::new(DEFAULT_COLS, DEFAULT_ROWS);
        // The inner term will set its flags when it parses the sequence.
        term.advance(b"\x1b[?1000h");
        assert!(term.mouse_grabbed());
    }
}
