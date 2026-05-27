//! Day 17 — spatial office compositor.
//!
//! Turns an [`OfficeState`] + decoded sprites into something a terminal can
//! show, with two outputs depending on the active tier:
//!
//! - **Cell tiers (T4/T5/T6/T6b)** — [`compose_cells_into`] paints a flat floor
//!   layer plus z-sorted furniture (real sprites) and character placeholders into
//!   a Ratatui [`Buffer`] using the half-block rasterizer.
//! - **Image tiers (T1-K/T1-O/T2/T3)** — [`image_placements`] returns where each
//!   sprite goes on the cell grid (cell + sub-cell), and [`compose_t1k_frame`]
//!   serialises a Kitty unicode-placeholder frame for those placements. (T2/T3
//!   frame composers reuse the same placement list with their own encoders; not
//!   wired into the live loop yet — see app.rs.)
//!
//! Coordinate model: a single [`View`] maps world pixels → screen cells. Camera
//! centers on the followed agent, else the office centre. Half-block geometry: 1
//! sprite-px column = 1 cell column, 2 sprite-px rows = 1 cell row. Sprites are
//! nearest-neighbour upscaled by `zoom` before rasterizing (cell tiers don't
//! scale beyond zoom 4 — they're pixel-painty by nature, arch §591).
//!
//! Out of scope for Day 17 (later phases): bubbles, matrix spawn/despawn,
//! selection outlines, seat indicators (Day 18-21); floor HSL colorization;
//! edit-mode overlays (Phase 5). Floor/wall here are flat colours; real floor
//! sprites arrive when the asset request path covers them (Day 18+).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::assets::AssetStore;
use crate::office::state::OfficeState;
use crate::office::types::{
    CharacterState, TileType, BUBBLE_SITTING_OFFSET_PX, BUBBLE_VERTICAL_OFFSET_PX,
};
use crate::render::bubbles::bubble_sprite;
use crate::render::cells::rasterize_halfblock;
use crate::render::char_sprites::CharSpriteStore;
use crate::render::kitty::{
    compute_placement, cursor_to, encode_transmit, encode_virtual_placement, placeholder_text,
    Placement,
};

const TILE_SIZE: i32 = crate::office::types::TILE_SIZE;

/// Flat floor / wall colours (v1 — HSL tile colours land later).
const FLOOR_BG: Color = Color::Rgb(56, 56, 74);
const WALL_BG: Color = Color::Rgb(34, 34, 46);

/// Maps world pixels to screen cells for the office region (half-block geometry).
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub area: Rect,
    pub zoom: u16,
    /// Camera world point in device pixels (world × zoom) shown at viewport centre.
    pub cam_dev_x: i32,
    pub cam_dev_y: i32,
}

impl View {
    /// Build a view centred on the followed agent, else the office centre.
    pub fn new(office: &OfficeState, area: Rect, zoom: u16) -> Self {
        let (cam_wx, cam_wy) = camera_world(office);
        Self {
            area,
            zoom: zoom.max(1),
            cam_dev_x: (cam_wx * zoom as f32) as i32,
            cam_dev_y: (cam_wy * zoom as f32) as i32,
        }
    }

    /// World-pixel point → screen cell `(col, row)` for **cell tiers** (half-block:
    /// 1 sprite-px column = 1 cell column, 2 sprite-px rows = 1 cell row). May be
    /// off-screen (caller clips).
    pub fn world_to_cell(&self, wx: f32, wy: f32) -> (i32, i32) {
        let dev_x = (wx * self.zoom as f32) as i32;
        let dev_y = (wy * self.zoom as f32) as i32;
        let col = self.area.x as i32 + self.area.width as i32 / 2 + (dev_x - self.cam_dev_x);
        // 2 device rows per cell row (half-block).
        let row = self.area.y as i32 + self.area.height as i32 / 2 + (dev_y - self.cam_dev_y) / 2;
        (col, row)
    }

    /// World-pixel point → terminal **device pixel** `(x, y)` for image tiers
    /// (true-pixel geometry; cells are `cell_w × cell_h` px). Same camera as
    /// `world_to_cell` — both transforms live on `View` so a camera change flows
    /// to both. Caller derives cell + sub-cell via `compute_placement`.
    pub fn world_to_device_px(&self, wx: f32, wy: f32, cell_w: u16, cell_h: u16) -> (i32, i32) {
        let dev_x = (wx * self.zoom as f32) as i32;
        let dev_y = (wy * self.zoom as f32) as i32;
        let center_x = (self.area.x as i32 + self.area.width as i32 / 2) * cell_w as i32;
        let center_y = (self.area.y as i32 + self.area.height as i32 / 2) * cell_h as i32;
        (center_x + dev_x - self.cam_dev_x, center_y + dev_y - self.cam_dev_y)
    }
}

/// Camera target: followed agent's centre, else office centre (world pixels).
fn camera_world(office: &OfficeState) -> (f32, f32) {
    if let Some(id) = office.camera_follow_id {
        if let Some(ch) = office.characters.get(&id) {
            return (ch.x, ch.y);
        }
    }
    (
        (office.layout.cols * TILE_SIZE) as f32 / 2.0,
        (office.layout.rows * TILE_SIZE) as f32 / 2.0,
    )
}

/// Nearest-neighbour integer upscale of tightly-packed RGBA. Factor 1 = clone.
fn scale_rgba(rgba: &[u8], w: u32, h: u32, factor: u32) -> (Vec<u8>, u32, u32) {
    if factor <= 1 {
        return (rgba.to_vec(), w, h);
    }
    let (nw, nh) = (w * factor, h * factor);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let (sx, sy) = (x / factor, y / factor);
            let si = ((sy * w + sx) * 4) as usize;
            let di = ((y * nw + x) * 4) as usize;
            out[di..di + 4].copy_from_slice(&rgba[si..si + 4]);
        }
    }
    (out, nw, nh)
}

/// A drawable sprite collected for z-sorting (world top-left + z baseline).
struct Drawable<'a> {
    z_y: f32,
    world_x: f32,
    world_y: f32,
    sprite: Option<&'a crate::assets::DecodedAsset>,
    /// Owned RGBA frame (matrix spawn/despawn composite). Takes priority over
    /// `sprite` when present.
    owned: Option<crate::assets::DecodedAsset>,
    /// Placeholder colour when there's no sprite yet (characters pre-Day-18).
    placeholder: Option<Color>,
    placeholder_px: (u32, u32),
}

/// Collect furniture + characters as z-sorted drawables. Furniture z from its
/// instance; characters from `y + TILE/2 + 0.5` (matches the webview's character
/// z-bias). A character renders its real sprite frame once its `char_N` sheet has
/// arrived and the `(palette, hue_shift)` set is built; until then a
/// palette-tinted placeholder block stands in.
fn collect_drawables<'a>(
    office: &'a OfficeState,
    assets: &'a AssetStore,
    chars: &'a CharSpriteStore,
) -> Vec<Drawable<'a>> {
    let mut out: Vec<Drawable> = Vec::new();

    for f in &office.furniture {
        out.push(Drawable {
            z_y: f.z_y,
            world_x: (f.col * TILE_SIZE) as f32,
            world_y: (f.row * TILE_SIZE) as f32,
            sprite: assets.get(&f.type_id),
            owned: None,
            placeholder: None,
            placeholder_px: (TILE_SIZE as u32, TILE_SIZE as u32),
        });
    }

    for ch in office.characters.values() {
        let z_y = ch.y + TILE_SIZE as f32 / 2.0 + 0.5;
        // Sitting offset: shift down 6px in TYPE so the char sits in the chair.
        let sit = if ch.state == CharacterState::Type {
            crate::office::types::CHARACTER_SITTING_OFFSET_PX as f32
        } else {
            0.0
        };
        // World anchor: bottom-centre on (ch.x, ch.y + sit), sprite 16w × 32h.
        let world_x = ch.x - 8.0;
        let world_y = ch.y + sit - 32.0;
        match chars.get(ch.palette, ch.hue_shift).map(|set| set.frame(ch)) {
            Some(frame) => {
                // Selection outline: just behind the char, 1px white ring, when
                // this is the selected agent and no matrix effect is running
                // (webview skips the outline during spawn/despawn).
                if office.selected_agent_id == Some(ch.id) && ch.matrix_effect.is_none() {
                    out.push(Drawable {
                        z_y: z_y - 0.5,
                        world_x: world_x - 1.0,
                        world_y: world_y - 1.0,
                        sprite: None,
                        owned: Some(crate::render::outline::outline_sprite(frame)),
                        placeholder: None,
                        placeholder_px: (18, 34),
                    });
                }
                // Matrix spawn/despawn composites over the base frame.
                let owned = ch.matrix_effect.map(|kind| {
                    crate::render::matrix::render_matrix_frame(
                        frame,
                        kind,
                        ch.matrix_effect_timer,
                        &ch.matrix_effect_seeds,
                    )
                });
                out.push(Drawable {
                    z_y,
                    world_x,
                    world_y,
                    sprite: Some(frame),
                    owned,
                    placeholder: None,
                    placeholder_px: (16, 32),
                });
            }
            None => {
                let tint = PLACEHOLDER_PALETTE[(ch.palette as usize) % PLACEHOLDER_PALETTE.len()];
                out.push(Drawable {
                    z_y,
                    world_x,
                    world_y: ch.y + sit - 24.0, // placeholder is ~24px tall
                    sprite: None,
                    owned: None,
                    placeholder: Some(tint),
                    placeholder_px: (16, 24),
                });
            }
        }
    }

    out.sort_by(|a, b| a.z_y.partial_cmp(&b.z_y).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Distinct-ish placeholder tints per base palette index, used only as a fallback
/// before a character's `char_N.png` sheet has been received/decoded.
const PLACEHOLDER_PALETTE: [Color; 6] = [
    Color::Rgb(214, 122, 90),
    Color::Rgb(108, 168, 220),
    Color::Rgb(132, 196, 124),
    Color::Rgb(212, 180, 100),
    Color::Rgb(176, 132, 204),
    Color::Rgb(206, 116, 152),
];

/// Paint a flat colour into the cell rect a tile/marker occupies (half-block:
/// rows = device-rows / 2). Used for the floor layer and character placeholders.
fn fill_block(buf: &mut Buffer, col0: i32, row0: i32, dev_w: i32, dev_h: i32, color: Color) {
    let cols = dev_w.max(1);
    let rows = ((dev_h + 1) / 2).max(1); // ceil(dev_h/2); i32 div_ceil is unstable
    let a = buf.area;
    for r in 0..rows {
        for c in 0..cols {
            let (x, y) = (col0 + c, row0 + r);
            if x < a.left() as i32 || x >= a.right() as i32 || y < a.top() as i32 || y >= a.bottom() as i32 {
                continue;
            }
            let cell = &mut buf[(x as u16, y as u16)];
            cell.set_char(' ');
            cell.set_bg(color);
        }
    }
}

/// Compose the cell-tier office (T4/T5/T6) into `buf` within `view.area`.
pub fn compose_cells_into(
    buf: &mut Buffer,
    office: &OfficeState,
    assets: &AssetStore,
    chars: &CharSpriteStore,
    view: &View,
) {
    let zoom = view.zoom as i32;

    // ── Floor / wall layer ──────────────────────────────────────────────────
    for row in 0..office.layout.rows {
        for col in 0..office.layout.cols {
            let t = office.tile_map.tile_at(col, row);
            let color = match t {
                TileType::Void => continue,
                TileType::Wall => WALL_BG,
                _ => FLOOR_BG,
            };
            let (c0, r0) = view.world_to_cell((col * TILE_SIZE) as f32, (row * TILE_SIZE) as f32);
            fill_block(buf, c0, r0, TILE_SIZE * zoom, TILE_SIZE * zoom, color);
        }
    }

    // ── Z-sorted drawables ──────────────────────────────────────────────────
    for d in collect_drawables(office, assets, chars) {
        let (c0, r0) = view.world_to_cell(d.world_x, d.world_y);
        // Owned matrix frame wins over the static sprite when present.
        let sprite = d.owned.as_ref().or(d.sprite);
        match (sprite, d.placeholder) {
            (Some(sprite), _) => {
                let (scaled, sw, sh) = scale_rgba(&sprite.rgba, sprite.width, sprite.height, view.zoom as u32);
                rasterize_halfblock(&scaled, sw, sh, buf, c0.max(0) as u16, r0.max(0) as u16);
            }
            (None, Some(tint)) => {
                let (pw, ph) = d.placeholder_px;
                fill_block(buf, c0, r0, pw as i32 * zoom, ph as i32 * zoom, tint);
            }
            (None, None) => {}
        }
    }

    // ── Speech bubbles (always on top of characters) ─────────────────────────
    for ch in office.characters.values() {
        let Some(kind) = ch.bubble_type else { continue };
        if ch.matrix_effect.is_some() {
            continue; // hidden during spawn/despawn, matching state.rs
        }
        let sprite = bubble_sprite(kind);
        let sit = if ch.state == CharacterState::Type {
            BUBBLE_SITTING_OFFSET_PX as f32
        } else {
            0.0
        };
        // Centred above the head: bottom at ch.y + sit - BUBBLE_VERTICAL_OFFSET_PX
        // minus the sprite height and a 1px gap (mirrors the webview math).
        let world_x = ch.x - sprite.width as f32 / 2.0;
        let world_y = ch.y + sit - BUBBLE_VERTICAL_OFFSET_PX as f32 - sprite.height as f32 - 1.0;
        let (c0, r0) = view.world_to_cell(world_x, world_y);
        let (scaled, sw, sh) = scale_rgba(&sprite.rgba, sprite.width, sprite.height, view.zoom as u32);
        rasterize_halfblock(&scaled, sw, sh, buf, c0.max(0) as u16, r0.max(0) as u16);
    }
}

// ── Image tiers ───────────────────────────────────────────────────────────────

/// Where a sprite lands on the cell grid for image-tier placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlacement {
    pub image_id: u32,
    pub placement: Placement,
}

/// Compute z-sorted image placements for furniture (image tiers). Characters are
/// excluded until their sprites exist (Day 18). `cell_px` is the terminal's
/// reported cell size (true-pixel geometry, unlike the cell tiers).
pub fn image_placements(
    office: &OfficeState,
    assets: &AssetStore,
    view: &View,
    cell_w: u16,
    cell_h: u16,
) -> Vec<ImagePlacement> {
    let mut items: Vec<(f32, ImagePlacement)> = Vec::new();
    for f in &office.furniture {
        let Some(sprite) = assets.get(&f.type_id) else { continue };
        // Image-tier device-pixel origin (true-pixel geometry, row not halved).
        let (dev_x, dev_y) = view.world_to_device_px(
            (f.col * TILE_SIZE) as f32,
            (f.row * TILE_SIZE) as f32,
            cell_w,
            cell_h,
        );
        if dev_x < 0 || dev_y < 0 {
            continue;
        }
        let dev_w = sprite.width * view.zoom as u32;
        let dev_h = sprite.height * view.zoom as u32;
        let placement = compute_placement(cell_w, cell_h, dev_x as u32, dev_y as u32, dev_w, dev_h);
        items.push((
            f.z_y,
            ImagePlacement { image_id: crate::assets::string_asset_id(&f.type_id), placement },
        ));
    }
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    items.into_iter().map(|(_, p)| p).collect()
}

/// Serialise a Kitty (T1-K) frame for the given placements: transmit each unique
/// sprite once (already-uploaded ids skipped via `uploaded`), then position the
/// cursor and emit a virtual placement + placeholder cells per entity.
pub fn compose_t1k_frame(
    placements: &[ImagePlacement],
    assets: &AssetStore,
    uploaded: &mut std::collections::HashSet<u32>,
) -> Vec<u8> {
    let mut out = Vec::new();
    // 1) Transmit any sprite referenced here that hasn't been uploaded yet.
    let mut seen_this_frame = std::collections::HashSet::new();
    for p in placements {
        if !seen_this_frame.insert(p.image_id) {
            continue;
        }
        if uploaded.insert(p.image_id) {
            if let Some(sprite) = asset_by_image_id(assets, p.image_id) {
                out.extend_from_slice(&encode_transmit(p.image_id, sprite.width, sprite.height, &sprite.rgba));
            }
        }
    }
    // 2) Place each entity (cursor → virtual placement → placeholder grid).
    for p in placements {
        let pl = &p.placement;
        out.extend_from_slice(&cursor_to(pl.cell_col, pl.cell_row));
        out.extend_from_slice(&encode_virtual_placement(p.image_id, 1, pl.cols, pl.rows, pl.x_off, pl.y_off));
        out.extend_from_slice(placeholder_text(p.image_id, pl.cols, pl.rows).as_bytes());
    }
    out
}

/// Reverse-lookup a decoded asset by its djb2 image id (small N; linear scan).
fn asset_by_image_id(assets: &AssetStore, image_id: u32) -> Option<&crate::assets::DecodedAsset> {
    assets
        .iter()
        .find(|(id, _)| crate::assets::string_asset_id(id) == image_id)
        .map(|(_, a)| a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::catalog::FurnitureCatalog;
    use crate::office::types::{FurnitureInstance, OfficeLayout};

    /// Office: all-floor 5×5, one furniture "DESK" at (2,2).
    fn office_with_desk() -> OfficeState {
        let n = 25;
        let layout = OfficeLayout {
            version: 1,
            cols: 5,
            rows: 5,
            tiles: vec![TileType::Floor1; n],
            furniture: vec![],
            tile_colors: vec![None; n],
        };
        let mut s = OfficeState::new(FurnitureCatalog::empty(), layout, 1);
        s.furniture = vec![FurnitureInstance {
            uid: "u1".into(),
            type_id: "DESK".into(),
            col: 2,
            row: 2,
            z_y: 100.0,
        }];
        s
    }

    #[test]
    fn view_centers_camera_at_viewport_center() {
        let office = office_with_desk();
        let area = Rect::new(0, 0, 80, 24);
        let view = View::new(&office, area, 1);
        // The camera world point maps to the viewport centre cell.
        let (cam_wx, cam_wy) = camera_world(&office);
        let (col, row) = view.world_to_cell(cam_wx, cam_wy);
        assert_eq!(col, 40); // area.width / 2
        assert_eq!(row, 12); // area.height / 2
    }

    #[test]
    fn world_to_cell_halves_vertical_axis() {
        let office = office_with_desk();
        let view = View::new(&office, Rect::new(0, 0, 80, 24), 1);
        let (cx, cy) = camera_world(&office);
        // +16 world px right → +16 cells; +16 world px down → +8 cell rows.
        let (col, row) = view.world_to_cell(cx + 16.0, cy + 16.0);
        assert_eq!(col, 40 + 16);
        assert_eq!(row, 12 + 8);
    }

    #[test]
    fn scale_rgba_nearest_neighbour_doubles() {
        let (out, w, h) = scale_rgba(&[1, 2, 3, 4], 1, 1, 2);
        assert_eq!((w, h), (2, 2));
        assert_eq!(out.len(), 2 * 2 * 4);
        assert!(out.chunks(4).all(|p| p == [1, 2, 3, 4]));
    }

    #[test]
    fn compose_cells_paints_floor_and_furniture() {
        let mut office = office_with_desk();
        office.camera_follow_id = None;
        let mut assets = AssetStore::new();
        // Register a red DESK sprite so the furniture has pixels.
        {
            // Stuff a decoded asset directly via the public frame path would need a
            // PNG; instead exercise the compositor with an in-memory store.
        }
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let view = View::new(&office, area, 1);
        // Inject a sprite by going through a real PNG round-trip.
        let png = {
            let mut img = image::RgbaImage::new(16, 16);
            for px in img.pixels_mut() {
                *px = image::Rgba([200, 40, 40, 255]);
            }
            let mut c = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(img).write_to(&mut c, image::ImageFormat::Png).unwrap();
            c.into_inner()
        };
        let nid = assets.register_request("DESK");
        assets.on_frame(nid, 0, true, &png).unwrap();

        let chars = CharSpriteStore::new();
        compose_cells_into(&mut buf, &office, &assets, &chars, &view);

        // Floor was painted somewhere (at least one cell has the floor bg).
        let painted_floor = (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf[(x, y)].bg == FLOOR_BG);
        assert!(painted_floor, "expected floor cells painted");

        // Desk sprite (red) rasterized as half-block somewhere.
        let painted_desk = (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf[(x, y)].fg == Color::Rgb(200, 40, 40));
        assert!(painted_desk, "expected desk sprite rasterized");
    }

    #[test]
    fn compose_cells_paints_character_sprite_when_sheet_present() {
        let mut office = office_with_desk();
        office.camera_follow_id = None;
        office.add_agent(1, Some(0), Some(0), None, true);

        // Build a solid-magenta 112×96 char sheet for palette 0.
        let mut store = CharSpriteStore::new();
        let sheet = crate::assets::DecodedAsset {
            width: 112,
            height: 96,
            rgba: {
                let mut v = vec![0u8; 112 * 96 * 4];
                for px in v.chunks_exact_mut(4) {
                    px.copy_from_slice(&[255, 0, 255, 255]);
                }
                v
            },
        };
        store.ingest(0, sheet);
        store.ensure(0, 0);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let view = View::new(&office, area, 1);
        compose_cells_into(&mut buf, &office, &AssetStore::new(), &store, &view);

        // The character's magenta sprite rasterized somewhere (half-block fg).
        let painted = (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf[(x, y)].fg == Color::Rgb(255, 0, 255));
        assert!(painted, "expected character sprite rasterized");
    }

    /// Build a CharSpriteStore with a solid-magenta sheet for `palette`.
    fn magenta_store(palette: u8) -> CharSpriteStore {
        let mut store = CharSpriteStore::new();
        let mut rgba = vec![0u8; 112 * 96 * 4];
        for px in rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 255, 255]);
        }
        store.ingest(palette, crate::assets::DecodedAsset { width: 112, height: 96, rgba });
        store.ensure(palette, 0);
        store
    }

    #[test]
    fn compose_cells_paints_matrix_head_during_spawn() {
        let mut office = office_with_desk();
        office.camera_follow_id = None;
        office.add_agent(1, Some(0), Some(0), None, false); // spawn effect armed
        let store = magenta_store(0);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let view = View::new(&office, area, 1);
        compose_cells_into(&mut buf, &office, &AssetStore::new(), &store, &view);

        // Matrix head colour (#ccffcc) painted somewhere during the sweep.
        let painted = (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf[(x, y)].fg == Color::Rgb(0xcc, 0xff, 0xcc));
        assert!(painted, "expected matrix head rasterized during spawn");
    }

    #[test]
    fn compose_cells_paints_white_outline_for_selected_char() {
        let mut office = office_with_desk();
        office.camera_follow_id = None;
        office.add_agent(1, Some(0), Some(0), None, true); // skip spawn matrix
        office.selected_agent_id = Some(1);
        let store = magenta_store(0);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let view = View::new(&office, area, 1);
        compose_cells_into(&mut buf, &office, &AssetStore::new(), &store, &view);

        // White ring (#FFFFFF) painted around the magenta char.
        let painted = (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf[(x, y)].fg == Color::Rgb(255, 255, 255));
        assert!(painted, "expected white selection outline");
    }

    #[test]
    fn compose_cells_paints_waiting_bubble_above_char() {
        let mut office = office_with_desk();
        office.camera_follow_id = None;
        office.add_agent(1, Some(0), Some(0), None, true);
        office.show_waiting_bubble(1);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let view = View::new(&office, area, 1);
        compose_cells_into(&mut buf, &office, &AssetStore::new(), &CharSpriteStore::new(), &view);

        // Bubble border colour (#555566) painted somewhere (fg via half-block).
        let painted = (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf[(x, y)].fg == Color::Rgb(0x55, 0x55, 0x66));
        assert!(painted, "expected waiting bubble rasterized");
    }

    #[test]
    fn image_placements_sorted_by_z_and_id_matches_djb2() {
        let mut office = office_with_desk();
        office.furniture.push(FurnitureInstance {
            uid: "u2".into(),
            type_id: "LAMP".into(),
            col: 0,
            row: 0,
            z_y: 10.0, // lower z → earlier
        });
        let mut assets = AssetStore::new();
        for id in ["DESK", "LAMP"] {
            let png = {
                let img = image::RgbaImage::new(16, 16);
                let mut c = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgba8(img).write_to(&mut c, image::ImageFormat::Png).unwrap();
                c.into_inner()
            };
            let nid = assets.register_request(id);
            assets.on_frame(nid, 0, true, &png).unwrap();
        }
        let view = View::new(&office, Rect::new(0, 0, 80, 24), 1);
        let pls = image_placements(&office, &assets, &view, 8, 16);
        assert_eq!(pls.len(), 2);
        // LAMP (z 10) before DESK (z 100).
        assert_eq!(pls[0].image_id, crate::assets::string_asset_id("LAMP"));
        assert_eq!(pls[1].image_id, crate::assets::string_asset_id("DESK"));
    }

    #[test]
    fn t1k_frame_transmits_once_then_places() {
        let office = office_with_desk();
        let mut assets = AssetStore::new();
        let png = {
            let img = image::RgbaImage::new(16, 16);
            let mut c = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(img).write_to(&mut c, image::ImageFormat::Png).unwrap();
            c.into_inner()
        };
        let nid = assets.register_request("DESK");
        assets.on_frame(nid, 0, true, &png).unwrap();

        let view = View::new(&office, Rect::new(0, 0, 80, 24), 1);
        let pls = image_placements(&office, &assets, &view, 8, 16);
        let mut uploaded = std::collections::HashSet::new();

        let frame1 = String::from_utf8(compose_t1k_frame(&pls, &assets, &mut uploaded)).unwrap();
        assert!(frame1.contains("\x1b_Ga=t"), "first frame transmits the sprite");
        assert!(frame1.contains("\x1b_Ga=p,U=1"), "and places it");

        // Second frame: sprite already uploaded → no transmit, still places.
        let frame2 = String::from_utf8(compose_t1k_frame(&pls, &assets, &mut uploaded)).unwrap();
        assert!(!frame2.contains("\x1b_Ga=t"), "no re-transmit once uploaded");
        assert!(frame2.contains("\x1b_Ga=p,U=1"), "still places");
    }
}
