//! Phase 3 Day 8-9 — `worldSeed` determinism (arch §298, CRIT-3 verification).
//!
//! Each agent owns a wander RNG seeded `worldSeed ^ agentId`. Two clients that
//! share a `worldSeed` and replay the same agent set + tick sequence must
//! produce byte-identical character positions tick-for-tick — regardless of the
//! order in which agents were added.

use pixel_agents_tui::office::catalog::FurnitureCatalog;
use pixel_agents_tui::office::state::OfficeState;
use pixel_agents_tui::office::types::{OfficeLayout, TileType};

const SEED_A: u32 = 0xC0FFEE;
const SEED_B: u32 = 0x1234_5678;
const DT: f32 = 0.05; // fixed sim step (decoupled from real frame time)
const TICKS: usize = 600; // 30 s of simulated time — many wander cycles

/// A fully walkable `cols × rows` office (all Floor1, no furniture / seats).
/// Seatless agents go idle and wander, exercising the per-agent wander RNG.
fn open_floor(cols: i32, rows: i32) -> OfficeLayout {
    let n = (cols * rows) as usize;
    OfficeLayout {
        version: 1,
        cols,
        rows,
        tiles: vec![TileType::Floor1; n],
        furniture: Vec::new(),
        tile_colors: vec![None; n],
    }
}

/// Build a world, add `ids` (each with an explicit palette so the palette
/// fallback RNG is never touched), and make every agent inactive so it wanders.
fn world_with(seed: u32, ids: &[i32]) -> OfficeState {
    let mut s = OfficeState::new(FurnitureCatalog::empty(), open_floor(10, 10), seed);
    for &id in ids {
        // Explicit palette/hue keeps agent creation deterministic and avoids the
        // (order-dependent) pick_diverse_palette fallback path.
        s.add_agent(id, Some((id as u8) % 6), Some(0), None, true);
        s.set_agent_active(id, false); // sentinel → drops to Idle → wanders
    }
    s
}

/// Position snapshot per agent: (id, tile_col, tile_row, x, y).
fn snapshot(s: &OfficeState) -> Vec<(i32, i32, i32, f32, f32)> {
    s.characters
        .values()
        .map(|c| (c.id, c.tile_col, c.tile_row, c.x, c.y))
        .collect()
}

fn run(mut s: OfficeState) -> OfficeState {
    for _ in 0..TICKS {
        s.tick(DT);
    }
    s
}

#[test]
fn same_seed_same_agents_identical_positions() {
    let a = run(world_with(SEED_A, &[1, 2, 3]));
    let b = run(world_with(SEED_A, &[1, 2, 3]));
    assert_eq!(
        snapshot(&a),
        snapshot(&b),
        "same worldSeed must reproduce identical positions tick-for-tick"
    );
}

#[test]
fn add_order_does_not_affect_per_agent_motion() {
    // Per-agent seeding means agent N's walk is independent of when it joined.
    let forward = run(world_with(SEED_A, &[1, 2, 3]));
    let reverse = run(world_with(SEED_A, &[3, 2, 1]));

    // characters is a BTreeMap keyed by id, so snapshot() is already id-sorted —
    // identical regardless of insertion order.
    assert_eq!(
        snapshot(&forward),
        snapshot(&reverse),
        "agent motion must depend only on (worldSeed, agentId), not add order"
    );
}

#[test]
fn different_seed_diverges() {
    // Negative control: a bug that makes everything compare-equal (e.g. agents
    // never move) would pass the parity tests above. Different seeds must yield
    // different motion, proving the seed actually drives the wander.
    let a = run(world_with(SEED_A, &[1, 2, 3]));
    let b = run(world_with(SEED_B, &[1, 2, 3]));
    assert_ne!(
        snapshot(&a),
        snapshot(&b),
        "different worldSeed must produce different positions"
    );
}

#[test]
fn agents_actually_moved() {
    // Guards against the degenerate "identical because nothing happened" case:
    // confirm at least one agent left its spawn tile over the run.
    let start = world_with(SEED_A, &[1, 2, 3]);
    let start_snap = snapshot(&start);
    let end_snap = snapshot(&run(world_with(SEED_A, &[1, 2, 3])));
    assert_ne!(
        start_snap, end_snap,
        "expected wandering agents to change position over {TICKS} ticks"
    );
}
