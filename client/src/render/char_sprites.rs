//! Character sprite sheets → per-state animation frames (Phase 3 Day 18).
//!
//! The daemon ships one pre-coloured sheet per palette (`char_0.png`..`char_5.png`,
//! 112×96 each) over the asset blob channel. Each sheet is 3 direction rows
//! (down, up, right) × 7 frames (16×32). We slice it into the same
//! `walk`/`typing`/`reading` frame sets the webview builds in `spriteData.ts`,
//! derive LEFT by horizontally flipping RIGHT, and apply per-character hue shifts
//! lazily (cached by `(palette, hue_shift)`), mirroring `getCharacterSprites`.
//!
//! Frame selection ([`CharacterSpriteSet::frame`]) ports `getCharacterSprite`
//! verbatim — this is the parity surface against the webview FSM.

use std::collections::HashMap;

use crate::assets::DecodedAsset;
use crate::office::characters::is_reading_tool;
use crate::office::types::{Character, CharacterState, Direction};
use crate::render::colorize::adjust_rgba;

/// Sheet geometry (matches `assets/characters/char_N.png` + `CHAR_FRAMES_PER_ROW`).
const FRAME_W: u32 = 16;
const FRAME_H: u32 = 32;
const FRAMES_PER_ROW: u32 = 7;
const DIR_ROWS: u32 = 3; // down, up, right (left is flipped right)
const SHEET_W: u32 = FRAME_W * FRAMES_PER_ROW; // 112
const SHEET_H: u32 = FRAME_H * DIR_ROWS; // 96

/// Internal direction order for the per-direction frame arrays.
const DIR_DOWN: usize = 0;
const DIR_UP: usize = 1;
const DIR_RIGHT: usize = 2;
const DIR_LEFT: usize = 3;

fn dir_idx(d: Direction) -> usize {
    match d {
        Direction::Down => DIR_DOWN,
        Direction::Up => DIR_UP,
        Direction::Right => DIR_RIGHT,
        Direction::Left => DIR_LEFT,
    }
}

/// One palette+hue's worth of animation frames, indexed `[dir][frame]`.
pub struct CharacterSpriteSet {
    walk: [[DecodedAsset; 4]; 4],
    typing: [[DecodedAsset; 2]; 4],
    reading: [[DecodedAsset; 2]; 4],
}

impl CharacterSpriteSet {
    /// Select the frame for a character's current state/direction/animation
    /// frame. Ports `getCharacterSprite` (`engine/characters.ts`):
    /// TYPE → reading/typing pair by tool; WALK → 4-frame walk cycle; IDLE →
    /// the standing pose (`walk[1]`).
    pub fn frame(&self, ch: &Character) -> &DecodedAsset {
        let d = dir_idx(ch.dir);
        let f = ch.frame as usize;
        match ch.state {
            CharacterState::Type => {
                if is_reading_tool(ch.current_tool.as_deref().unwrap_or("")) {
                    &self.reading[d][f % 2]
                } else {
                    &self.typing[d][f % 2]
                }
            }
            CharacterState::Walk => &self.walk[d][f % 4],
            CharacterState::Idle => &self.walk[d][1],
        }
    }
}

/// Decoded sheets + lazily-built, hue-shifted frame sets.
#[derive(Default)]
pub struct CharSpriteStore {
    /// palette index → decoded 112×96 sheet.
    sheets: HashMap<u8, DecodedAsset>,
    /// (palette, hue_shift) → built frame set.
    cache: HashMap<(u8, i32), CharacterSpriteSet>,
}

impl CharSpriteStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The asset id the daemon serves a palette's sheet under (`char_<n>`).
    pub fn asset_id(palette: u8) -> String {
        format!("char_{palette}")
    }

    /// Parse `char_<n>` → palette index, or `None` for any other id.
    pub fn palette_of(asset_id: &str) -> Option<u8> {
        asset_id.strip_prefix("char_")?.parse().ok()
    }

    /// Store a decoded sheet for `palette`, dropping any frame sets built from a
    /// prior sheet so they rebuild on next `ensure`.
    pub fn ingest(&mut self, palette: u8, sheet: DecodedAsset) {
        self.sheets.insert(palette, sheet);
        self.cache.retain(|(p, _), _| *p != palette);
    }

    /// Ensure the `(palette, hue_shift)` frame set is built (no-op if the sheet
    /// is missing/undersized or the set already exists).
    pub fn ensure(&mut self, palette: u8, hue_shift: i32) {
        if self.cache.contains_key(&(palette, hue_shift)) {
            return;
        }
        let Some(sheet) = self.sheets.get(&palette) else { return };
        if sheet.width < SHEET_W || sheet.height < SHEET_H {
            return; // non-standard sheet — caller falls back to a placeholder
        }
        let set = build_set(sheet, hue_shift);
        self.cache.insert((palette, hue_shift), set);
    }

    /// Get a previously-`ensure`d frame set, if present.
    pub fn get(&self, palette: u8, hue_shift: i32) -> Option<&CharacterSpriteSet> {
        self.cache.get(&(palette, hue_shift))
    }

    pub fn has_sheet(&self, palette: u8) -> bool {
        self.sheets.contains_key(&palette)
    }
}

/// Slice a 16×32 frame at sheet grid `(col, row)` out of a sheet, hue-shifting it.
fn extract_frame(sheet: &DecodedAsset, col: u32, row: u32, hue_shift: i32) -> DecodedAsset {
    let (x0, y0) = (col * FRAME_W, row * FRAME_H);
    let mut rgba = vec![0u8; (FRAME_W * FRAME_H * 4) as usize];
    for y in 0..FRAME_H {
        let src_row = ((y0 + y) * sheet.width + x0) as usize * 4;
        let dst_row = (y * FRAME_W) as usize * 4;
        let len = (FRAME_W * 4) as usize;
        rgba[dst_row..dst_row + len].copy_from_slice(&sheet.rgba[src_row..src_row + len]);
    }
    if hue_shift != 0 {
        adjust_rgba(&mut rgba, hue_shift as f32, 0.0, 0.0, 0.0);
    }
    DecodedAsset { width: FRAME_W, height: FRAME_H, rgba }
}

/// Flip a 16×32 frame horizontally (LEFT = mirrored RIGHT).
fn flip_h(src: &DecodedAsset) -> DecodedAsset {
    let w = src.width;
    let h = src.height;
    let mut rgba = vec![0u8; src.rgba.len()];
    for y in 0..h {
        for x in 0..w {
            let sx = (w - 1 - x) as usize;
            let si = ((y * w) as usize + sx) * 4;
            let di = ((y * w + x) as usize) * 4;
            rgba[di..di + 4].copy_from_slice(&src.rgba[si..si + 4]);
        }
    }
    DecodedAsset { width: w, height: h, rgba }
}

/// Build a full frame set from a sheet at one hue shift. Mirrors the assembly in
/// `getCharacterSprites`: walk = frames [0,1,2,1]; typing = [3,4]; reading =
/// [5,6]; per direction. Rows: 0=down, 1=up, 2=right; LEFT flips RIGHT.
fn build_set(sheet: &DecodedAsset, hue_shift: i32) -> CharacterSpriteSet {
    // base[row][col] for the three source rows.
    let frame = |col: u32, row: u32| extract_frame(sheet, col, row, hue_shift);
    let row_frames = |row: u32| -> [DecodedAsset; 7] {
        std::array::from_fn(|c| frame(c as u32, row))
    };
    let down = row_frames(0);
    let up = row_frames(1);
    let right = row_frames(2);

    let walk_from = |r: &[DecodedAsset; 7]| -> [DecodedAsset; 4] {
        [r[0].clone(), r[1].clone(), r[2].clone(), r[1].clone()]
    };
    let pair = |r: &[DecodedAsset; 7], a: usize, b: usize| -> [DecodedAsset; 2] {
        [r[a].clone(), r[b].clone()]
    };
    let flip4 = |a: &[DecodedAsset; 4]| -> [DecodedAsset; 4] {
        [flip_h(&a[0]), flip_h(&a[1]), flip_h(&a[2]), flip_h(&a[3])]
    };
    let flip2 = |a: &[DecodedAsset; 2]| -> [DecodedAsset; 2] {
        [flip_h(&a[0]), flip_h(&a[1])]
    };

    let walk_right = walk_from(&right);
    let typing_right = pair(&right, 3, 4);
    let reading_right = pair(&right, 5, 6);

    let mut walk: [[DecodedAsset; 4]; 4] = Default::default();
    let mut typing: [[DecodedAsset; 2]; 4] = Default::default();
    let mut reading: [[DecodedAsset; 2]; 4] = Default::default();

    walk[DIR_DOWN] = walk_from(&down);
    walk[DIR_UP] = walk_from(&up);
    walk[DIR_LEFT] = flip4(&walk_right);
    walk[DIR_RIGHT] = walk_right;

    typing[DIR_DOWN] = pair(&down, 3, 4);
    typing[DIR_UP] = pair(&up, 3, 4);
    typing[DIR_LEFT] = flip2(&typing_right);
    typing[DIR_RIGHT] = typing_right;

    reading[DIR_DOWN] = pair(&down, 5, 6);
    reading[DIR_UP] = pair(&up, 5, 6);
    reading[DIR_LEFT] = flip2(&reading_right);
    reading[DIR_RIGHT] = reading_right;

    CharacterSpriteSet { walk, typing, reading }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 112×96 sheet where every pixel encodes its `(col_frame, row)` so slices
    /// are identifiable: R = frame-col index × 10, G = row × 10, B = 0, A = 255.
    fn marker_sheet() -> DecodedAsset {
        let mut rgba = vec![0u8; (SHEET_W * SHEET_H * 4) as usize];
        for y in 0..SHEET_H {
            for x in 0..SHEET_W {
                let i = ((y * SHEET_W + x) as usize) * 4;
                let frame_col = (x / FRAME_W) as u8;
                let row = (y / FRAME_H) as u8;
                rgba[i] = frame_col * 10;
                rgba[i + 1] = row * 10;
                rgba[i + 2] = 0;
                rgba[i + 3] = 255;
            }
        }
        DecodedAsset { width: SHEET_W, height: SHEET_H, rgba }
    }

    fn ch(state: CharacterState, dir: Direction, frame: u8, tool: Option<&str>) -> Character {
        let mut c = crate::office::characters::create_character(1, 0, 0, None, None, 1);
        c.state = state;
        c.dir = dir;
        c.frame = frame;
        c.current_tool = tool.map(|s| s.to_string());
        c
    }

    #[test]
    fn extract_frame_picks_correct_grid_cell() {
        let sheet = marker_sheet();
        // frame col 5, row 2 (right) → R=50, G=20.
        let f = extract_frame(&sheet, 5, 2, 0);
        assert_eq!((f.width, f.height), (FRAME_W, FRAME_H));
        assert_eq!(&f.rgba[0..4], &[50, 20, 0, 255]);
    }

    #[test]
    fn flip_h_mirrors_columns() {
        let sheet = marker_sheet();
        let f = extract_frame(&sheet, 0, 0, 0);
        let flipped = flip_h(&f);
        // Pixel (0,0) of the flip == pixel (15,0) of the source.
        let src_last = &f.rgba[(15 * 4)..(15 * 4 + 4)];
        assert_eq!(&flipped.rgba[0..4], src_last);
    }

    #[test]
    fn frame_select_matches_webview_mapping() {
        let sheet = marker_sheet();
        let set = build_set(&sheet, 0);

        // WALK, down, frame 5 → walk[down][5 % 4 = 1] → source frame col 1.
        let f = set.frame(&ch(CharacterState::Walk, Direction::Down, 5, None));
        assert_eq!(f.rgba[0], 10, "walk frame col 1");

        // TYPE + Read tool, down → reading[down][0] → source frame col 5.
        let f = set.frame(&ch(CharacterState::Type, Direction::Down, 0, Some("Read")));
        assert_eq!(f.rgba[0], 50, "reading frame col 5");

        // TYPE + Edit tool, down → typing[down][0] → source frame col 3.
        let f = set.frame(&ch(CharacterState::Type, Direction::Down, 0, Some("Edit")));
        assert_eq!(f.rgba[0], 30, "typing frame col 3");

        // IDLE → standing pose walk[1] → source frame col 1.
        let f = set.frame(&ch(CharacterState::Idle, Direction::Down, 0, None));
        assert_eq!(f.rgba[0], 10, "idle uses walk frame col 1");
    }

    #[test]
    fn left_dir_uses_flipped_right() {
        let sheet = marker_sheet();
        let set = build_set(&sheet, 0);
        // Right walk frame 0 col 0; its flip's first pixel == right frame's last col pixel.
        let right = set.frame(&ch(CharacterState::Walk, Direction::Right, 0, None));
        let left = set.frame(&ch(CharacterState::Walk, Direction::Left, 0, None));
        let right_last = &right.rgba[(15 * 4)..(15 * 4 + 4)];
        assert_eq!(&left.rgba[0..4], right_last);
    }

    #[test]
    fn store_builds_and_caches_set() {
        let mut store = CharSpriteStore::new();
        assert!(store.get(0, 0).is_none());
        store.ingest(0, marker_sheet());
        store.ensure(0, 0);
        assert!(store.get(0, 0).is_some());
        // Hue-shifted variant is a distinct cache entry.
        store.ensure(0, 90);
        assert!(store.get(0, 90).is_some());
    }

    #[test]
    fn ingest_invalidates_prior_sets() {
        let mut store = CharSpriteStore::new();
        store.ingest(0, marker_sheet());
        store.ensure(0, 0);
        store.ingest(0, marker_sheet()); // reload
        assert!(store.get(0, 0).is_none(), "reload drops built sets");
    }

    #[test]
    fn undersized_sheet_does_not_build() {
        let mut store = CharSpriteStore::new();
        store.ingest(0, DecodedAsset { width: 16, height: 16, rgba: vec![0; 16 * 16 * 4] });
        store.ensure(0, 0);
        assert!(store.get(0, 0).is_none());
    }

    #[test]
    fn asset_id_roundtrips_palette() {
        assert_eq!(CharSpriteStore::asset_id(3), "char_3");
        assert_eq!(CharSpriteStore::palette_of("char_3"), Some(3));
        assert_eq!(CharSpriteStore::palette_of("DESK"), None);
    }
}
