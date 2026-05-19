#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use rand::Rng;

use crate::office::tile_map::{find_path, TileMap};
use crate::office::types::*;

// ── Tool classification ───────────────────────────────────────────────────────

const READING_TOOLS: &[&str] = &["Read", "Grep", "Glob", "WebFetch", "WebSearch"];

pub fn is_reading_tool(tool: &str) -> bool {
    READING_TOOLS.contains(&tool)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn tile_center(col: i32, row: i32) -> (f32, f32) {
    (
        (col * TILE_SIZE + TILE_SIZE / 2) as f32,
        (row * TILE_SIZE + TILE_SIZE / 2) as f32,
    )
}

fn direction_between(from_col: i32, from_row: i32, to_col: i32, to_row: i32) -> Direction {
    let dc = to_col - from_col;
    let dr = to_row - from_row;
    if dc > 0 {
        Direction::Right
    } else if dc < 0 {
        Direction::Left
    } else if dr > 0 {
        Direction::Down
    } else {
        Direction::Up
    }
}

// ── Character creation ────────────────────────────────────────────────────────

/// Create a new character. Starts in TYPE state (active) at the given seat, or
/// at (1,1) if no seat provided. `rng` seeds the initial wander_limit.
pub fn create_character(
    id: i32,
    palette: u8,
    hue_shift: i32,
    seat_id: Option<String>,
    seat: Option<&Seat>,
    rng: &mut impl Rng,
) -> Character {
    let (col, row, dir) = seat
        .map(|s| (s.seat_col, s.seat_row, s.facing_dir))
        .unwrap_or((1, 1, Direction::Down));
    let (cx, cy) = tile_center(col, row);

    Character {
        id,
        palette,
        hue_shift,
        x: cx,
        y: cy,
        tile_col: col,
        tile_row: row,
        dir,
        state: CharacterState::Type,
        frame: 0,
        frame_timer: 0.0,
        seat_id,
        path: Vec::new(),
        move_progress: 0.0,
        current_tool: None,
        wander_timer: 0.0,
        wander_count: 0,
        wander_limit: rng.gen_range(WANDER_MOVES_BEFORE_REST_MIN..=WANDER_MOVES_BEFORE_REST_MAX),
        is_active: true,
        bubble_type: None,
        bubble_timer: 0.0,
        seat_timer: 0.0,
        is_subagent: false,
        parent_agent_id: None,
        matrix_effect: None,
        matrix_effect_timer: 0.0,
        matrix_effect_seeds: [0.0; 16],
    }
}

// ── Character FSM ─────────────────────────────────────────────────────────────

/// Advance one character's FSM by `dt` seconds.
///
/// `blocked` must already have the character's own seat tile removed so BFS can
/// path to it. The caller (OfficeState::tick) handles that carve-out.
pub fn update_character(
    ch: &mut Character,
    dt: f32,
    walkable_tiles: &[(i32, i32)],
    seats: &BTreeMap<String, Seat>,
    tile_map: &TileMap,
    blocked: &BTreeSet<(i32, i32)>,
    rng: &mut impl Rng,
) {
    ch.frame_timer += dt;

    match ch.state {
        // ── TYPE ─────────────────────────────────────────────────────────────
        CharacterState::Type => {
            if ch.frame_timer >= TYPE_FRAME_DURATION_SEC {
                ch.frame_timer -= TYPE_FRAME_DURATION_SEC;
                ch.frame = (ch.frame + 1) % 2;
            }
            if !ch.is_active {
                // seatTimer > 0: count down before standing up
                if ch.seat_timer > 0.0 {
                    ch.seat_timer -= dt;
                    return;
                }
                // seatTimer <= 0 (including -1 sentinel): stand up immediately
                ch.seat_timer = 0.0;
                ch.state = CharacterState::Idle;
                ch.frame = 0;
                ch.frame_timer = 0.0;
                ch.wander_timer = rng.gen_range(WANDER_PAUSE_MIN_SEC..WANDER_PAUSE_MAX_SEC);
                ch.wander_count = 0;
                ch.wander_limit =
                    rng.gen_range(WANDER_MOVES_BEFORE_REST_MIN..=WANDER_MOVES_BEFORE_REST_MAX);
            }
        }

        // ── IDLE ──────────────────────────────────────────────────────────────
        CharacterState::Idle => {
            ch.frame = 0;
            if ch.seat_timer < 0.0 { ch.seat_timer = 0.0; } // clear -1 sentinel

            if ch.is_active {
                // Became active — pathfind to seat
                let seat_info: Option<(i32, i32, Direction)> = ch
                    .seat_id
                    .as_ref()
                    .and_then(|sid| seats.get(sid))
                    .map(|s| (s.seat_col, s.seat_row, s.facing_dir));

                match seat_info {
                    None => {
                        // No seat — type in place
                        ch.state = CharacterState::Type;
                        ch.frame = 0;
                        ch.frame_timer = 0.0;
                    }
                    Some((sc, sr, facing)) => {
                        let path =
                            find_path(ch.tile_col, ch.tile_row, sc, sr, tile_map, blocked);
                        if !path.is_empty() {
                            ch.path = path;
                            ch.move_progress = 0.0;
                            ch.state = CharacterState::Walk;
                            ch.frame = 0;
                            ch.frame_timer = 0.0;
                        } else {
                            // Already at seat or no path — sit down
                            ch.state = CharacterState::Type;
                            ch.dir = facing;
                            ch.frame = 0;
                            ch.frame_timer = 0.0;
                        }
                    }
                }
                return;
            }

            // Inactive: countdown wander timer
            ch.wander_timer -= dt;
            if ch.wander_timer <= 0.0 {
                // Return to seat if wandered enough
                if ch.wander_count >= ch.wander_limit {
                    let seat_info: Option<(i32, i32)> = ch
                        .seat_id
                        .as_ref()
                        .and_then(|sid| seats.get(sid))
                        .map(|s| (s.seat_col, s.seat_row));

                    if let Some((sc, sr)) = seat_info {
                        let path = find_path(ch.tile_col, ch.tile_row, sc, sr, tile_map, blocked);
                        if !path.is_empty() {
                            ch.path = path;
                            ch.move_progress = 0.0;
                            ch.state = CharacterState::Walk;
                            ch.frame = 0;
                            ch.frame_timer = 0.0;
                            ch.wander_timer =
                                rng.gen_range(WANDER_PAUSE_MIN_SEC..WANDER_PAUSE_MAX_SEC);
                            return;
                        }
                    }
                }

                // Wander to a random walkable tile
                if !walkable_tiles.is_empty() {
                    let idx = rng.gen_range(0..walkable_tiles.len());
                    let target = walkable_tiles[idx];
                    let path = find_path(
                        ch.tile_col,
                        ch.tile_row,
                        target.0,
                        target.1,
                        tile_map,
                        blocked,
                    );
                    if !path.is_empty() {
                        ch.path = path;
                        ch.move_progress = 0.0;
                        ch.state = CharacterState::Walk;
                        ch.frame = 0;
                        ch.frame_timer = 0.0;
                        ch.wander_count += 1;
                    }
                }
                ch.wander_timer = rng.gen_range(WANDER_PAUSE_MIN_SEC..WANDER_PAUSE_MAX_SEC);
            }
        }

        // ── WALK ──────────────────────────────────────────────────────────────
        CharacterState::Walk => {
            if ch.frame_timer >= WALK_FRAME_DURATION_SEC {
                ch.frame_timer -= WALK_FRAME_DURATION_SEC;
                ch.frame = (ch.frame + 1) % 4;
            }

            if ch.path.is_empty() {
                // Path complete — snap to tile center and transition
                let (cx, cy) = tile_center(ch.tile_col, ch.tile_row);
                ch.x = cx;
                ch.y = cy;

                if ch.is_active {
                    let seat_info: Option<(i32, i32, Direction)> = ch
                        .seat_id
                        .as_ref()
                        .and_then(|sid| seats.get(sid))
                        .map(|s| (s.seat_col, s.seat_row, s.facing_dir));

                    match seat_info {
                        None => { ch.state = CharacterState::Type; }
                        Some((sc, sr, facing)) => {
                            if ch.tile_col == sc && ch.tile_row == sr {
                                ch.state = CharacterState::Type;
                                ch.dir = facing;
                            } else {
                                ch.state = CharacterState::Idle;
                            }
                        }
                    }
                } else {
                    // Check if arrived at assigned seat
                    let arrived: Option<(i32, i32, Direction)> = ch
                        .seat_id
                        .as_ref()
                        .and_then(|sid| seats.get(sid))
                        .filter(|s| ch.tile_col == s.seat_col && ch.tile_row == s.seat_row)
                        .map(|s| (s.seat_col, s.seat_row, s.facing_dir));

                    if let Some((_, _, facing)) = arrived {
                        ch.state = CharacterState::Type;
                        ch.dir = facing;
                        ch.seat_timer = if ch.seat_timer < 0.0 {
                            0.0
                        } else {
                            rng.gen_range(SEAT_REST_MIN_SEC..SEAT_REST_MAX_SEC)
                        };
                        ch.wander_count = 0;
                        ch.wander_limit =
                            rng.gen_range(WANDER_MOVES_BEFORE_REST_MIN..=WANDER_MOVES_BEFORE_REST_MAX);
                        ch.frame = 0;
                        ch.frame_timer = 0.0;
                        return;
                    } else {
                        ch.state = CharacterState::Idle;
                        ch.wander_timer =
                            rng.gen_range(WANDER_PAUSE_MIN_SEC..WANDER_PAUSE_MAX_SEC);
                    }
                }
                ch.frame = 0;
                ch.frame_timer = 0.0;
                return;
            }

            // Advance along path
            let next = ch.path[0];
            ch.dir = direction_between(ch.tile_col, ch.tile_row, next.0, next.1);
            ch.move_progress += (WALK_SPEED_PX_PER_SEC / TILE_SIZE as f32) * dt;

            let (from_cx, from_cy) = tile_center(ch.tile_col, ch.tile_row);
            let (to_cx, to_cy) = tile_center(next.0, next.1);
            let t = ch.move_progress.min(1.0);
            ch.x = from_cx + (to_cx - from_cx) * t;
            ch.y = from_cy + (to_cy - from_cy) * t;

            if ch.move_progress >= 1.0 {
                ch.tile_col = next.0;
                ch.tile_row = next.1;
                ch.x = to_cx;
                ch.y = to_cy;
                ch.path.remove(0);
                ch.move_progress = 0.0;
            }

            // Repath to seat if became active while wandering
            if ch.is_active {
                let seat_info: Option<(i32, i32)> = ch
                    .seat_id
                    .as_ref()
                    .and_then(|sid| seats.get(sid))
                    .map(|s| (s.seat_col, s.seat_row));

                if let Some((sc, sr)) = seat_info {
                    let already_heading = ch
                        .path
                        .last()
                        .map(|&(lc, lr)| lc == sc && lr == sr)
                        .unwrap_or(false);
                    if !already_heading {
                        let new_path = find_path(
                            ch.tile_col,
                            ch.tile_row,
                            sc,
                            sr,
                            tile_map,
                            blocked,
                        );
                        if !new_path.is_empty() {
                            ch.path = new_path;
                            ch.move_progress = 0.0;
                        }
                    }
                }
            }
        }
    }
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

    fn make_seat(uid: &str, col: i32, row: i32) -> Seat {
        Seat { uid: uid.to_string(), seat_col: col, seat_row: row, facing_dir: Direction::Down, assigned: true }
    }

    #[test]
    fn create_character_starts_type_active() {
        let mut rng = seeded();
        let seat = make_seat("s1", 2, 2);
        let ch = create_character(1, 0, 0, Some("s1".into()), Some(&seat), &mut rng);
        assert_eq!(ch.state, CharacterState::Type);
        assert!(ch.is_active);
        assert_eq!(ch.tile_col, 2);
        assert_eq!(ch.tile_row, 2);
    }

    #[test]
    fn type_inactive_sentinel_transitions_to_idle_immediately() {
        let mut rng = seeded();
        let seat = make_seat("s1", 2, 2);
        let mut ch = create_character(1, 0, 0, Some("s1".into()), Some(&seat), &mut rng);
        ch.is_active = false;
        ch.seat_timer = -1.0; // sentinel from setAgentActive(false)

        let seats = BTreeMap::new();
        let tile_map = crate::office::tile_map::TileMap::new(1, 1, vec![TileType::Void]);
        let blocked = BTreeSet::new();
        update_character(&mut ch, 0.016, &[], &seats, &tile_map, &blocked, &mut rng);

        assert_eq!(ch.state, CharacterState::Idle);
        assert_eq!(ch.seat_timer, 0.0);
    }

    #[test]
    fn seat_timer_positive_delays_idle_transition() {
        let mut rng = seeded();
        let seat = make_seat("s1", 2, 2);
        let mut ch = create_character(1, 0, 0, Some("s1".into()), Some(&seat), &mut rng);
        ch.is_active = false;
        ch.seat_timer = 1.0; // 1 second rest remaining

        let seats = BTreeMap::new();
        let tile_map = crate::office::tile_map::TileMap::new(1, 1, vec![TileType::Void]);
        let blocked = BTreeSet::new();
        update_character(&mut ch, 0.016, &[], &seats, &tile_map, &blocked, &mut rng);

        // Still TYPE, timer decremented
        assert_eq!(ch.state, CharacterState::Type);
        assert!(ch.seat_timer < 1.0 && ch.seat_timer > 0.0);
    }
}
