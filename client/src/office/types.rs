#![allow(dead_code)]

use rand::rngs::SmallRng;

// ── Animation / simulation constants ─────────────────────────────────────────

pub const TILE_SIZE: i32 = 16;
pub const WALK_SPEED_PX_PER_SEC: f32 = 48.0;
pub const WALK_FRAME_DURATION_SEC: f32 = 0.15;
pub const TYPE_FRAME_DURATION_SEC: f32 = 0.3;
pub const WANDER_PAUSE_MIN_SEC: f32 = 2.0;
pub const WANDER_PAUSE_MAX_SEC: f32 = 20.0;
pub const WANDER_MOVES_BEFORE_REST_MIN: i32 = 3;
pub const WANDER_MOVES_BEFORE_REST_MAX: i32 = 6;
pub const SEAT_REST_MIN_SEC: f32 = 120.0;
pub const SEAT_REST_MAX_SEC: f32 = 240.0;
pub const MATRIX_EFFECT_DURATION: f32 = 0.3;
pub const WAITING_BUBBLE_DURATION_SEC: f32 = 2.0;
pub const DISMISS_BUBBLE_FAST_FADE_SEC: f32 = 0.3;
pub const INACTIVE_SEAT_TIMER_MIN_SEC: f32 = 3.0;
pub const INACTIVE_SEAT_TIMER_RANGE_SEC: f32 = 2.0;
pub const HUE_SHIFT_MIN_DEG: i32 = 45;
pub const HUE_SHIFT_RANGE_DEG: i32 = 271;
pub const AUTO_ON_FACING_DEPTH: i32 = 3;
pub const AUTO_ON_SIDE_DEPTH: i32 = 2;
pub const CHARACTER_HIT_HALF_WIDTH: i32 = 8;
pub const CHARACTER_HIT_HEIGHT: i32 = 24;
pub const CHARACTER_SITTING_OFFSET_PX: i32 = 6;
pub const FURNITURE_ANIM_INTERVAL_SEC: f32 = 0.2;
pub const MATRIX_SPRITE_COLS: usize = 16;
pub const NUM_PALETTES: usize = 6;

// ── Tile type ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TileType {
    Wall = 0,
    Floor1 = 1,
    Floor2 = 2,
    Floor3 = 3,
    Floor4 = 4,
    Floor5 = 5,
    Floor6 = 6,
    Floor7 = 7,
    Floor8 = 8,
    Floor9 = 9,
    Void = 255,
}

impl TileType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => TileType::Wall,
            1 => TileType::Floor1,
            2 => TileType::Floor2,
            3 => TileType::Floor3,
            4 => TileType::Floor4,
            5 => TileType::Floor5,
            6 => TileType::Floor6,
            7 => TileType::Floor7,
            8 => TileType::Floor8,
            9 => TileType::Floor9,
            _ => TileType::Void,
        }
    }

    pub fn is_floor(self) -> bool {
        (self as u8) >= 1 && (self as u8) <= 9
    }
}

// ── Direction ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Down,
    Left,
    Right,
    Up,
}

// ── Character state ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterState {
    Idle,
    Walk,
    Type,
}

// ── Bubble ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BubbleType {
    Permission,
    Waiting,
}

// ── Matrix effect ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixEffectKind {
    Spawn,
    Despawn,
}

// ── Seat ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Seat {
    pub uid: String,
    pub seat_col: i32,
    pub seat_row: i32,
    pub facing_dir: Direction,
    pub assigned: bool,
}

// ── Character ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Character {
    pub id: i32,
    pub palette: u8,
    pub hue_shift: i32,
    pub x: f32,
    pub y: f32,
    pub tile_col: i32,
    pub tile_row: i32,
    pub dir: Direction,
    pub state: CharacterState,
    pub frame: u8,
    pub frame_timer: f32,
    pub seat_id: Option<String>,
    pub path: Vec<(i32, i32)>,
    pub move_progress: f32,
    pub current_tool: Option<String>,
    pub wander_timer: f32,
    pub wander_count: i32,
    pub wander_limit: i32,
    pub is_active: bool,
    pub bubble_type: Option<BubbleType>,
    pub bubble_timer: f32,
    /// -1.0 sentinel: turn just ended → skip next long seat rest.
    pub seat_timer: f32,
    pub is_subagent: bool,
    pub parent_agent_id: Option<i32>,
    pub matrix_effect: Option<MatrixEffectKind>,
    pub matrix_effect_timer: f32,
    pub matrix_effect_seeds: [f32; 16],
    /// Per-agent wander RNG, seeded `worldSeed ^ agentId` (arch §298). Drives all
    /// random FSM decisions (wander targets, pause/rest timers, matrix seeds) so
    /// that any two clients sharing `worldSeed` reproduce identical motion for a
    /// given agent id — independent of how many other agents exist or join order.
    pub rng: SmallRng,
}

/// Seed a per-agent RNG from the world seed and agent id (arch §298).
///
/// `agent_id` is reinterpreted as `u32` (negative sub-agent ids included) before
/// the XOR so the mapping is total and collision-free across the id space.
pub fn agent_rng_seed(world_seed: u32, agent_id: i32) -> u64 {
    (world_seed ^ (agent_id as u32)) as u64
}

// ── Furniture ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct TileColor {
    pub h: f32,
    pub s: f32,
    pub b: f32,
    pub c: f32,
}

#[derive(Debug, Clone)]
pub struct FurnitureColor {
    pub h: f32,
    pub s: f32,
    pub b: f32,
    pub c: f32,
    pub colorize: bool,
}

#[derive(Debug, Clone)]
pub struct PlacedFurniture {
    pub uid: String,
    pub type_id: String,
    pub col: i32,
    pub row: i32,
    pub color: Option<FurnitureColor>,
}

/// Day 4-7: position + zY only; sprite data added in Day 10-16.
#[derive(Debug, Clone)]
pub struct FurnitureInstance {
    pub uid: String,
    pub type_id: String,
    pub col: i32,
    pub row: i32,
    pub z_y: f32,
}

// ── Office layout ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OfficeLayout {
    pub version: u32,
    pub cols: i32,
    pub rows: i32,
    pub tiles: Vec<TileType>,
    pub furniture: Vec<PlacedFurniture>,
    pub tile_colors: Vec<Option<TileColor>>,
}

impl OfficeLayout {
    pub fn empty(cols: i32, rows: i32) -> Self {
        let n = (cols * rows) as usize;
        Self {
            version: 1,
            cols,
            rows,
            tiles: vec![TileType::Void; n],
            furniture: Vec::new(),
            tile_colors: vec![None; n],
        }
    }
}
