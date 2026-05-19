#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use rand::Rng;

use crate::office::catalog::FurnitureCatalog;
use crate::office::characters::{create_character, tile_center, update_character};
use crate::office::layout::{
    get_blocked_tiles, layout_to_furniture_instances, layout_to_seats, layout_to_tile_map,
};
use crate::office::tile_map::{get_walkable_tiles, TileMap};
use crate::office::types::*;
use crate::render::matrix::matrix_effect_seeds;

// ── OfficeState ───────────────────────────────────────────────────────────────

pub struct OfficeState {
    pub layout: OfficeLayout,
    pub catalog: FurnitureCatalog,
    pub tile_map: TileMap,
    pub seats: BTreeMap<String, Seat>,
    pub blocked_tiles: BTreeSet<(i32, i32)>,
    pub walkable_tiles: Vec<(i32, i32)>,
    pub furniture: Vec<FurnitureInstance>,
    /// Keyed by agent ID (negative for sub-agents). BTreeMap for deterministic iteration.
    pub characters: BTreeMap<i32, Character>,
    pub furniture_anim_timer: f32,
    pub selected_agent_id: Option<i32>,
    pub camera_follow_id: Option<i32>,
    subagent_id_map: BTreeMap<String, i32>,
    subagent_meta: BTreeMap<i32, SubagentMeta>,
    next_subagent_id: i32,
}

struct SubagentMeta {
    parent_agent_id: i32,
    parent_tool_id: String,
}

impl OfficeState {
    pub fn new(catalog: FurnitureCatalog, layout: OfficeLayout) -> Self {
        let tile_map = layout_to_tile_map(&layout);
        let seats = layout_to_seats(&layout.furniture, &catalog);
        let blocked = get_blocked_tiles(&layout.furniture, &catalog, None);
        let walkable = get_walkable_tiles(&tile_map, &blocked);
        let furniture = layout_to_furniture_instances(&layout.furniture, &catalog);

        Self {
            layout,
            catalog,
            tile_map,
            seats,
            blocked_tiles: blocked,
            walkable_tiles: walkable,
            furniture,
            characters: BTreeMap::new(),
            furniture_anim_timer: 0.0,
            selected_agent_id: None,
            camera_follow_id: None,
            subagent_id_map: BTreeMap::new(),
            subagent_meta: BTreeMap::new(),
            next_subagent_id: -1,
        }
    }

    // ── Layout rebuilding ─────────────────────────────────────────────────────

    pub fn rebuild_from_layout(&mut self, layout: OfficeLayout, shift: Option<(i32, i32)>) {
        self.layout = layout;
        self.tile_map = layout_to_tile_map(&self.layout);
        self.seats = layout_to_seats(&self.layout.furniture, &self.catalog);
        self.blocked_tiles = get_blocked_tiles(&self.layout.furniture, &self.catalog, None);
        self.furniture = layout_to_furniture_instances(&self.layout.furniture, &self.catalog);
        self.walkable_tiles = get_walkable_tiles(&self.tile_map, &self.blocked_tiles);

        if let Some((sc, sr)) = shift {
            if sc != 0 || sr != 0 {
                for ch in self.characters.values_mut() {
                    ch.tile_col += sc;
                    ch.tile_row += sr;
                    ch.x += (sc * TILE_SIZE) as f32;
                    ch.y += (sr * TILE_SIZE) as f32;
                    ch.path.clear();
                    ch.move_progress = 0.0;
                }
            }
        }

        // Reset all seat assignments
        for seat in self.seats.values_mut() {
            seat.assigned = false;
        }

        let ids: Vec<i32> = self.characters.keys().copied().collect();

        // First pass: keep characters at their existing seats
        for &id in &ids {
            let seat_id = self.characters[&id].seat_id.clone();
            if let Some(ref sid) = seat_id {
                let seat_info = self.seats.get(sid).filter(|s| !s.assigned)
                    .map(|s| (s.seat_col, s.seat_row, s.facing_dir));
                if let Some((sc, sr, facing)) = seat_info {
                    self.seats.get_mut(sid).unwrap().assigned = true;
                    let (cx, cy) = tile_center(sc, sr);
                    let ch = self.characters.get_mut(&id).unwrap();
                    ch.tile_col = sc;
                    ch.tile_row = sr;
                    ch.x = cx;
                    ch.y = cy;
                    ch.dir = facing;
                    continue;
                }
            }
            self.characters.get_mut(&id).unwrap().seat_id = None;
        }

        // Second pass: assign remaining characters to any free seat
        for &id in &ids {
            if self.characters[&id].seat_id.is_some() { continue; }
            if let Some(sid) = find_free_seat_uid(&self.seats) {
                let seat = self.seats.get_mut(&sid).unwrap();
                seat.assigned = true;
                let (sc, sr, facing) = (seat.seat_col, seat.seat_row, seat.facing_dir);
                let (cx, cy) = tile_center(sc, sr);
                let ch = self.characters.get_mut(&id).unwrap();
                ch.seat_id = Some(sid);
                ch.tile_col = sc;
                ch.tile_row = sr;
                ch.x = cx;
                ch.y = cy;
                ch.dir = facing;
            }
        }
    }

    // ── Agent management ──────────────────────────────────────────────────────

    pub fn add_agent(
        &mut self,
        id: i32,
        preferred_palette: Option<u8>,
        preferred_hue_shift: Option<i32>,
        preferred_seat_id: Option<&str>,
        skip_spawn_effect: bool,
        rng: &mut impl Rng,
    ) {
        if self.characters.contains_key(&id) { return; }

        let (palette, hue_shift) = if let Some(p) = preferred_palette {
            (p, preferred_hue_shift.unwrap_or(0))
        } else {
            pick_diverse_palette(&self.characters, rng)
        };

        let seat_id = preferred_seat_id
            .filter(|sid| self.seats.get(*sid).map_or(false, |s| !s.assigned))
            .map(|s| s.to_string())
            .or_else(|| find_free_seat_uid(&self.seats));

        let mut ch = if let Some(ref sid) = seat_id {
            let seat_info = {
                let seat = self.seats.get_mut(sid).unwrap();
                seat.assigned = true;
                Seat {
                    uid: seat.uid.clone(),
                    seat_col: seat.seat_col,
                    seat_row: seat.seat_row,
                    facing_dir: seat.facing_dir,
                    assigned: true,
                }
            };
            create_character(id, palette, hue_shift, Some(sid.clone()), Some(&seat_info), rng)
        } else {
            let spawn = self.walkable_tiles.first().copied().unwrap_or((1, 1));
            let mut c = create_character(id, palette, hue_shift, None, None, rng);
            let (cx, cy) = tile_center(spawn.0, spawn.1);
            c.x = cx;
            c.y = cy;
            c.tile_col = spawn.0;
            c.tile_row = spawn.1;
            c
        };

        if !skip_spawn_effect {
            ch.matrix_effect = Some(MatrixEffectKind::Spawn);
            ch.matrix_effect_timer = 0.0;
            ch.matrix_effect_seeds = matrix_effect_seeds(rng);
        }

        self.characters.insert(id, ch);
    }

    pub fn remove_agent(&mut self, id: i32, rng: &mut impl Rng) {
        let Some(ch) = self.characters.get_mut(&id) else { return };
        if ch.matrix_effect == Some(MatrixEffectKind::Despawn) { return; }

        if let Some(ref sid) = ch.seat_id {
            if let Some(seat) = self.seats.get_mut(sid) {
                seat.assigned = false;
            }
        }
        if self.selected_agent_id == Some(id) { self.selected_agent_id = None; }
        if self.camera_follow_id == Some(id) { self.camera_follow_id = None; }

        ch.matrix_effect = Some(MatrixEffectKind::Despawn);
        ch.matrix_effect_timer = 0.0;
        ch.matrix_effect_seeds = matrix_effect_seeds(rng);
        ch.bubble_type = None;
    }

    pub fn set_agent_active(&mut self, id: i32, active: bool) {
        let Some(ch) = self.characters.get_mut(&id) else { return };
        ch.is_active = active;
        if !active {
            ch.seat_timer = -1.0; // sentinel: skip next seat rest
            ch.path.clear();
            ch.move_progress = 0.0;
        }
        self.rebuild_furniture_instances();
    }

    pub fn set_agent_tool(&mut self, id: i32, tool: Option<String>) {
        if let Some(ch) = self.characters.get_mut(&id) {
            ch.current_tool = tool;
        }
    }

    // ── Bubble management ─────────────────────────────────────────────────────

    pub fn show_permission_bubble(&mut self, id: i32) {
        if let Some(ch) = self.characters.get_mut(&id) {
            ch.bubble_type = Some(BubbleType::Permission);
            ch.bubble_timer = 0.0;
        }
    }

    pub fn clear_permission_bubble(&mut self, id: i32) {
        if let Some(ch) = self.characters.get_mut(&id) {
            if ch.bubble_type == Some(BubbleType::Permission) {
                ch.bubble_type = None;
                ch.bubble_timer = 0.0;
            }
        }
    }

    pub fn show_waiting_bubble(&mut self, id: i32) {
        if let Some(ch) = self.characters.get_mut(&id) {
            ch.bubble_type = Some(BubbleType::Waiting);
            ch.bubble_timer = WAITING_BUBBLE_DURATION_SEC;
        }
    }

    pub fn dismiss_bubble(&mut self, id: i32) {
        if let Some(ch) = self.characters.get_mut(&id) {
            match ch.bubble_type {
                Some(BubbleType::Permission) => {
                    ch.bubble_type = None;
                    ch.bubble_timer = 0.0;
                }
                Some(BubbleType::Waiting) => {
                    ch.bubble_timer = ch.bubble_timer.min(DISMISS_BUBBLE_FAST_FADE_SEC);
                }
                None => {}
            }
        }
    }

    // ── Sub-agent management ──────────────────────────────────────────────────

    pub fn add_subagent(
        &mut self,
        parent_agent_id: i32,
        parent_tool_id: &str,
        rng: &mut impl Rng,
    ) -> i32 {
        let key = format!("{}:{}", parent_agent_id, parent_tool_id);
        if let Some(&existing) = self.subagent_id_map.get(&key) {
            return existing;
        }

        let id = self.next_subagent_id;
        self.next_subagent_id -= 1;

        let (palette, hue_shift, p_col, p_row, p_dir) = self
            .characters
            .get(&parent_agent_id)
            .map(|p| (p.palette, p.hue_shift, p.tile_col, p.tile_row, p.dir))
            .unwrap_or((0, 0, 0, 0, Direction::Down));

        let occupied: BTreeSet<(i32, i32)> =
            self.characters.values().map(|c| (c.tile_col, c.tile_row)).collect();

        let spawn = self
            .walkable_tiles
            .iter()
            .filter(|&&(c, r)| !occupied.contains(&(c, r)))
            .min_by_key(|&&(c, r)| (c - p_col).abs() + (r - p_row).abs())
            .copied()
            .unwrap_or((p_col, p_row));

        let mut ch = create_character(id, palette, hue_shift, None, None, rng);
        let (cx, cy) = tile_center(spawn.0, spawn.1);
        ch.x = cx;
        ch.y = cy;
        ch.tile_col = spawn.0;
        ch.tile_row = spawn.1;
        ch.dir = p_dir;
        ch.is_subagent = true;
        ch.parent_agent_id = Some(parent_agent_id);
        ch.matrix_effect = Some(MatrixEffectKind::Spawn);
        ch.matrix_effect_timer = 0.0;
        ch.matrix_effect_seeds = matrix_effect_seeds(rng);

        self.characters.insert(id, ch);
        self.subagent_id_map.insert(key, id);
        self.subagent_meta.insert(id, SubagentMeta {
            parent_agent_id,
            parent_tool_id: parent_tool_id.to_string(),
        });
        id
    }

    pub fn remove_subagent(&mut self, parent_agent_id: i32, parent_tool_id: &str, rng: &mut impl Rng) {
        let key = format!("{}:{}", parent_agent_id, parent_tool_id);
        let Some(&id) = self.subagent_id_map.get(&key) else { return };

        if let Some(ch) = self.characters.get_mut(&id) {
            if ch.matrix_effect == Some(MatrixEffectKind::Despawn) {
                self.subagent_id_map.remove(&key);
                self.subagent_meta.remove(&id);
                return;
            }
            ch.matrix_effect = Some(MatrixEffectKind::Despawn);
            ch.matrix_effect_timer = 0.0;
            ch.matrix_effect_seeds = matrix_effect_seeds(rng);
            ch.bubble_type = None;
        }
        self.subagent_id_map.remove(&key);
        self.subagent_meta.remove(&id);
        if self.selected_agent_id == Some(id) { self.selected_agent_id = None; }
        if self.camera_follow_id == Some(id) { self.camera_follow_id = None; }
    }

    pub fn get_subagent_id(&self, parent_agent_id: i32, parent_tool_id: &str) -> Option<i32> {
        self.subagent_id_map
            .get(&format!("{}:{}", parent_agent_id, parent_tool_id))
            .copied()
    }

    // ── Simulation tick ───────────────────────────────────────────────────────

    /// Advance the simulation by `dt` seconds.
    pub fn tick(&mut self, dt: f32, rng: &mut impl Rng) {
        // Furniture animation frame cycling
        let prev = (self.furniture_anim_timer / FURNITURE_ANIM_INTERVAL_SEC) as u32;
        self.furniture_anim_timer += dt;
        let next = (self.furniture_anim_timer / FURNITURE_ANIM_INTERVAL_SEC) as u32;
        if next != prev {
            self.rebuild_furniture_instances();
        }

        let ids: Vec<i32> = self.characters.keys().copied().collect();
        let mut to_delete: Vec<i32> = Vec::new();

        // First pass: advance matrix effects and bubble timers
        for &id in &ids {
            let Some(ch) = self.characters.get_mut(&id) else { continue };

            if ch.matrix_effect.is_some() {
                ch.matrix_effect_timer += dt;
                if ch.matrix_effect_timer >= MATRIX_EFFECT_DURATION {
                    if ch.matrix_effect == Some(MatrixEffectKind::Spawn) {
                        ch.matrix_effect = None;
                        ch.matrix_effect_timer = 0.0;
                    } else {
                        to_delete.push(id);
                    }
                }
                continue; // skip FSM while matrix effect active
            }

            if ch.bubble_type == Some(BubbleType::Waiting) {
                ch.bubble_timer -= dt;
                if ch.bubble_timer <= 0.0 {
                    ch.bubble_type = None;
                    ch.bubble_timer = 0.0;
                }
            }
        }

        // Second pass: run FSM with own-seat temporarily unblocked
        for &id in &ids {
            if to_delete.contains(&id) { continue; }
            let Some(ch) = self.characters.get(&id) else { continue };
            if ch.matrix_effect.is_some() { continue; }

            // Carve out own seat so pathfinding can reach it
            let own_seat = ch
                .seat_id
                .as_ref()
                .and_then(|sid| self.seats.get(sid))
                .map(|s| (s.seat_col, s.seat_row));

            let mut blocked = self.blocked_tiles.clone();
            if let Some(key) = own_seat { blocked.remove(&key); }

            let ch = self.characters.get_mut(&id).unwrap();
            update_character(
                ch,
                dt,
                &self.walkable_tiles,
                &self.seats,
                &self.tile_map,
                &blocked,
                rng,
            );
        }

        for id in to_delete {
            self.characters.remove(&id);
        }
    }

    // ── Hit testing ───────────────────────────────────────────────────────────

    pub fn get_character_at(&self, world_x: f32, world_y: f32) -> Option<i32> {
        let mut chars: Vec<&Character> = self
            .characters
            .values()
            .filter(|ch| ch.matrix_effect != Some(MatrixEffectKind::Despawn))
            .collect();
        chars.sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal));

        for ch in chars {
            let offset =
                if ch.state == CharacterState::Type { CHARACTER_SITTING_OFFSET_PX as f32 } else { 0.0 };
            let ay = ch.y + offset;
            let hw = CHARACTER_HIT_HALF_WIDTH as f32;
            let h = CHARACTER_HIT_HEIGHT as f32;
            if world_x >= ch.x - hw
                && world_x <= ch.x + hw
                && world_y >= ay - h
                && world_y <= ay
            {
                return Some(ch.id);
            }
        }
        None
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn rebuild_furniture_instances(&mut self) {
        self.furniture = layout_to_furniture_instances(&self.layout.furniture, &self.catalog);
    }
}

// ── Free functions ────────────────────────────────────────────────────────────

fn find_free_seat_uid(seats: &BTreeMap<String, Seat>) -> Option<String> {
    seats.iter().find(|(_, s)| !s.assigned).map(|(uid, _)| uid.clone())
}

/// Pick the least-used palette, with random hue shift for repeats.
fn pick_diverse_palette(characters: &BTreeMap<i32, Character>, rng: &mut impl Rng) -> (u8, i32) {
    let mut counts = [0usize; NUM_PALETTES];
    for ch in characters.values() {
        if !ch.is_subagent && (ch.palette as usize) < NUM_PALETTES {
            counts[ch.palette as usize] += 1;
        }
    }
    let min_count = *counts.iter().min().unwrap_or(&0);
    let available: Vec<usize> = counts
        .iter()
        .enumerate()
        .filter(|(_, &c)| c == min_count)
        .map(|(i, _)| i)
        .collect();
    let palette = available[rng.gen_range(0..available.len())] as u8;
    let hue_shift = if min_count > 0 {
        rng.gen_range(HUE_SHIFT_MIN_DEG..HUE_SHIFT_MIN_DEG + HUE_SHIFT_RANGE_DEG)
    } else {
        0
    };
    (palette, hue_shift)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn seeded() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn empty_state() -> OfficeState {
        OfficeState::new(FurnitureCatalog::empty(), OfficeLayout::empty(5, 5))
    }

    #[test]
    fn add_and_remove_agent() {
        let mut s = empty_state();
        let mut rng = seeded();
        s.add_agent(1, Some(0), Some(0), None, true, &mut rng);
        assert!(s.characters.contains_key(&1));
        s.remove_agent(1, &mut rng);
        // Character still present (despawn animation pending)
        assert!(s.characters.contains_key(&1));
        assert_eq!(s.characters[&1].matrix_effect, Some(MatrixEffectKind::Despawn));
    }

    #[test]
    fn tick_removes_despawned_character() {
        let mut s = empty_state();
        let mut rng = seeded();
        s.add_agent(1, Some(0), Some(0), None, true, &mut rng);
        s.remove_agent(1, &mut rng);
        // Advance past despawn duration
        s.tick(MATRIX_EFFECT_DURATION + 0.01, &mut rng);
        assert!(!s.characters.contains_key(&1));
    }

    #[test]
    fn pick_diverse_palette_first_six_unique() {
        let mut rng = seeded();
        let mut s = empty_state();
        let mut palettes = Vec::new();
        for id in 1..=6 {
            s.add_agent(id, None, None, None, true, &mut rng);
            palettes.push(s.characters[&id].palette);
        }
        // Each palette 0-5 should appear exactly once
        let mut sorted = palettes.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 6);
    }

    #[test]
    fn set_agent_active_false_sets_sentinel() {
        let mut s = empty_state();
        let mut rng = seeded();
        s.add_agent(1, Some(0), Some(0), None, true, &mut rng);
        s.set_agent_active(1, false);
        assert_eq!(s.characters[&1].seat_timer, -1.0);
        assert!(s.characters[&1].path.is_empty());
    }

    #[test]
    fn subagent_dedup() {
        let mut s = empty_state();
        let mut rng = seeded();
        s.add_agent(1, Some(0), Some(0), None, true, &mut rng);
        let id1 = s.add_subagent(1, "tool-A", &mut rng);
        let id2 = s.add_subagent(1, "tool-A", &mut rng);
        assert_eq!(id1, id2);
    }

    #[test]
    fn waiting_bubble_fades() {
        let mut s = empty_state();
        let mut rng = seeded();
        s.add_agent(1, Some(0), Some(0), None, true, &mut rng);
        s.show_waiting_bubble(1);
        assert_eq!(s.characters[&1].bubble_type, Some(BubbleType::Waiting));
        s.tick(WAITING_BUBBLE_DURATION_SEC + 0.01, &mut rng);
        assert_eq!(s.characters[&1].bubble_type, None);
    }
}
