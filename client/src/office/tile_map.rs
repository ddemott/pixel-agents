#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::office::types::TileType;

// ── TileMap ───────────────────────────────────────────────────────────────────

pub struct TileMap {
    pub cols: i32,
    pub rows: i32,
    tiles: Vec<TileType>,
}

impl TileMap {
    pub fn new(cols: i32, rows: i32, tiles: Vec<TileType>) -> Self {
        Self { cols, rows, tiles }
    }

    pub fn tile_at(&self, col: i32, row: i32) -> TileType {
        if col < 0 || row < 0 || col >= self.cols || row >= self.rows {
            return TileType::Void;
        }
        self.tiles[(row * self.cols + col) as usize]
    }
}

// ── Walkability ───────────────────────────────────────────────────────────────

pub fn is_walkable(col: i32, row: i32, tile_map: &TileMap, blocked: &BTreeSet<(i32, i32)>) -> bool {
    let t = tile_map.tile_at(col, row);
    if matches!(t, TileType::Wall | TileType::Void) {
        return false;
    }
    !blocked.contains(&(col, row))
}

pub fn get_walkable_tiles(tile_map: &TileMap, blocked: &BTreeSet<(i32, i32)>) -> Vec<(i32, i32)> {
    let mut result = Vec::new();
    for row in 0..tile_map.rows {
        for col in 0..tile_map.cols {
            if is_walkable(col, row, tile_map, blocked) {
                result.push((col, row));
            }
        }
    }
    result
}

// ── BFS pathfinding ───────────────────────────────────────────────────────────

/// BFS on 4-connected grid. Returns path excluding start, including end.
pub fn find_path(
    start_col: i32,
    start_row: i32,
    end_col: i32,
    end_row: i32,
    tile_map: &TileMap,
    blocked: &BTreeSet<(i32, i32)>,
) -> Vec<(i32, i32)> {
    if start_col == end_col && start_row == end_row {
        return vec![];
    }
    if !is_walkable(end_col, end_row, tile_map, blocked) {
        return vec![];
    }

    let start = (start_col, start_row);
    let end = (end_col, end_row);
    let mut visited: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut parent: BTreeMap<(i32, i32), (i32, i32)> = BTreeMap::new();
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();

    visited.insert(start);
    queue.push_back(start);

    const DIRS: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

    'bfs: while let Some(cur) = queue.pop_front() {
        if cur == end {
            break 'bfs;
        }
        for (dc, dr) in DIRS {
            let next = (cur.0 + dc, cur.1 + dr);
            if !visited.contains(&next) && is_walkable(next.0, next.1, tile_map, blocked) {
                visited.insert(next);
                parent.insert(next, cur);
                queue.push_back(next);
            }
        }
    }

    if !visited.contains(&end) {
        return vec![];
    }

    // Reconstruct path
    let mut path = Vec::new();
    let mut node = end;
    while node != start {
        path.push(node);
        node = *parent.get(&node).unwrap();
    }
    path.reverse();
    path
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::types::TileType;

    fn simple_map() -> TileMap {
        // 5×5 grid: all Floor1 except borders (Wall)
        let mut tiles = vec![TileType::Floor1; 25];
        for col in 0..5 {
            tiles[col] = TileType::Wall;          // row 0
            tiles[4 * 5 + col] = TileType::Wall;  // row 4
        }
        for row in 0..5 {
            tiles[row * 5] = TileType::Wall;           // col 0
            tiles[row * 5 + 4] = TileType::Wall;       // col 4
        }
        TileMap::new(5, 5, tiles)
    }

    #[test]
    fn same_start_end_returns_empty() {
        let m = simple_map();
        let b = BTreeSet::new();
        assert!(find_path(2, 2, 2, 2, &m, &b).is_empty());
    }

    #[test]
    fn unreachable_returns_empty() {
        let m = simple_map();
        let b = BTreeSet::new();
        // Wall tile is not walkable → no path
        assert!(find_path(1, 1, 0, 0, &m, &b).is_empty());
    }

    #[test]
    fn straight_path() {
        let m = simple_map();
        let b = BTreeSet::new();
        let path = find_path(1, 1, 3, 1, &m, &b);
        // Should step (2,1) → (3,1)
        assert!(!path.is_empty());
        assert_eq!(*path.last().unwrap(), (3, 1));
    }

    #[test]
    fn blocked_tile_avoided() {
        let m = simple_map();
        let mut b = BTreeSet::new();
        b.insert((2, 1)); // block direct route
        let path = find_path(1, 1, 3, 1, &m, &b);
        assert!(!path.is_empty());
        assert!(!path.contains(&(2, 1)));
    }

    #[test]
    fn is_walkable_wall_false() {
        let m = simple_map();
        let b = BTreeSet::new();
        assert!(!is_walkable(0, 0, &m, &b)); // corner wall
    }

    #[test]
    fn is_walkable_floor_true() {
        let m = simple_map();
        let b = BTreeSet::new();
        assert!(is_walkable(2, 2, &m, &b));
    }
}
