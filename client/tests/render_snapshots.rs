//! Phase 3 Day 22 — render snapshot matrix.
//!
//! Snapshots the cell-tier (half-block) compositor — the live office render
//! path — across the scene matrix: empty office, a character, speech bubbles,
//! selection outline, and the matrix spawn/despawn effect. Each snapshot is the
//! buffer's glyph grid, which pins entity geometry/placement (the parity surface
//! against the webview). Per-pixel colour is covered by the `render::*` unit
//! tests; image-tier output is covered by the byte-level tests in
//! `render::{kitty,iterm2,sixel}`.
//!
//! Regenerate after intentional changes with `INSTA_UPDATE=always cargo test
//! --test render_snapshots` (or `cargo insta accept`).
//!
//! Scope/limits: the synthetic character sheet is a flat left-half-opaque block,
//! so character-only scenes collapse to a uniform `▀` silhouette — the snapshots
//! pin *placement* and the geometry of varying-shape overlays (bubble shapes,
//! the half-rendered matrix sweep), NOT full sprite silhouette nor the presence
//! of the selection outline (the outline scene's glyph grid is identical to the
//! unselected one — only colour differs, which the `render::outline`/`scene`
//! unit tests assert directly). They are a geometry/regression net layered on
//! top of the colour-exact unit tests, not a replacement for them.

use pixel_agents_tui::assets::{AssetStore, DecodedAsset};
use pixel_agents_tui::office::catalog::FurnitureCatalog;
use pixel_agents_tui::office::state::OfficeState;
use pixel_agents_tui::office::types::{MatrixEffectKind, OfficeLayout, TileType};
use pixel_agents_tui::render::char_sprites::CharSpriteStore;
use pixel_agents_tui::render::scene::{compose_cells_into, View};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const AREA: Rect = Rect { x: 0, y: 0, width: 40, height: 18 };

/// A 7×5 all-floor office, camera fixed on centre (no followed agent).
fn office() -> OfficeState {
    let (cols, rows) = (7, 5);
    let n = (cols * rows) as usize;
    let layout = OfficeLayout {
        version: 1,
        cols,
        rows,
        tiles: vec![TileType::Floor1; n],
        furniture: vec![],
        tile_colors: vec![None; n],
    };
    let mut s = OfficeState::new(FurnitureCatalog::empty(), layout, 1);
    s.camera_follow_id = None;
    s
}

/// CharSpriteStore with a deterministic non-uniform sheet for `palette`: a
/// vertical split (left half opaque, right half transparent) so half-block
/// glyphs vary and silhouette/flip is visible in the grid.
fn store(palette: u8) -> CharSpriteStore {
    let (w, h) = (112u32, 96u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            // Opaque on the left 8px of every 16px frame column.
            if x % 16 < 8 {
                rgba[i..i + 4].copy_from_slice(&[200, 60, 200, 255]);
            }
        }
    }
    let mut s = CharSpriteStore::new();
    s.ingest(palette, DecodedAsset { width: w, height: h, rgba });
    s.ensure(palette, 0);
    s
}

/// Render a scene to its glyph grid (trailing blanks trimmed per row).
fn render(office: &OfficeState, chars: &CharSpriteStore) -> String {
    let mut buf = Buffer::empty(AREA);
    let view = View::new(office, AREA, 1);
    compose_cells_into(&mut buf, office, &AssetStore::new(), chars, &view);
    let mut lines = Vec::new();
    for y in 0..AREA.height {
        let mut row = String::new();
        for x in 0..AREA.width {
            row.push_str(buf[(x, y)].symbol());
        }
        lines.push(row.trim_end().to_string());
    }
    lines.join("\n")
}

#[test]
fn snapshot_empty_office() {
    let o = office();
    insta::assert_snapshot!(render(&o, &CharSpriteStore::new()));
}

#[test]
fn snapshot_single_character() {
    let mut o = office();
    o.add_agent(1, Some(0), Some(0), None, true); // skip spawn matrix
    insta::assert_snapshot!(render(&o, &store(0)));
}

#[test]
fn snapshot_waiting_bubble() {
    let mut o = office();
    o.add_agent(1, Some(0), Some(0), None, true);
    o.show_waiting_bubble(1);
    insta::assert_snapshot!(render(&o, &store(0)));
}

#[test]
fn snapshot_permission_bubble() {
    let mut o = office();
    o.add_agent(1, Some(0), Some(0), None, true);
    o.show_permission_bubble(1);
    insta::assert_snapshot!(render(&o, &store(0)));
}

#[test]
fn snapshot_selection_outline() {
    let mut o = office();
    o.add_agent(1, Some(0), Some(0), None, true);
    o.selected_agent_id = Some(1);
    insta::assert_snapshot!(render(&o, &store(0)));
}

#[test]
fn snapshot_matrix_spawn_midway() {
    let mut o = office();
    o.add_agent(1, Some(0), Some(0), None, false); // spawn effect armed
    let ch = o.characters.get_mut(&1).unwrap();
    ch.matrix_effect = Some(MatrixEffectKind::Spawn);
    ch.matrix_effect_timer = 0.15; // mid-sweep
    ch.matrix_effect_seeds = [0.0; 16];
    insta::assert_snapshot!(render(&o, &store(0)));
}

#[test]
fn snapshot_matrix_despawn_midway() {
    let mut o = office();
    o.add_agent(1, Some(0), Some(0), None, true);
    let ch = o.characters.get_mut(&1).unwrap();
    ch.matrix_effect = Some(MatrixEffectKind::Despawn);
    ch.matrix_effect_timer = 0.15;
    ch.matrix_effect_seeds = [0.0; 16];
    insta::assert_snapshot!(render(&o, &store(0)));
}
