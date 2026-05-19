#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::office::catalog::FurnitureCatalog;
use crate::office::tile_map::TileMap;
use crate::office::types::{
    Direction, FurnitureColor, FurnitureInstance, OfficeLayout, PlacedFurniture, Seat, TileColor,
    TileType, TILE_SIZE,
};

// ── TileMap ───────────────────────────────────────────────────────────────────

pub fn layout_to_tile_map(layout: &OfficeLayout) -> TileMap {
    TileMap::new(layout.cols, layout.rows, layout.tiles.clone())
}

// ── Blocked tiles ─────────────────────────────────────────────────────────────

/// Returns tiles blocked by furniture footprints, skipping backgroundTiles rows.
/// `exclude` tiles are not added to the result (used to carve out seat tiles).
pub fn get_blocked_tiles(
    furniture: &[PlacedFurniture],
    catalog: &FurnitureCatalog,
    exclude: Option<&BTreeSet<(i32, i32)>>,
) -> BTreeSet<(i32, i32)> {
    let mut tiles = BTreeSet::new();
    for item in furniture {
        let Some(entry) = catalog.get(&item.type_id) else { continue };
        let bg = entry.background_tiles;
        for dr in 0..entry.footprint_h {
            if dr < bg { continue; }
            for dc in 0..entry.footprint_w {
                let key = (item.col + dc, item.row + dr);
                if exclude.map_or(true, |ex| !ex.contains(&key)) {
                    tiles.insert(key);
                }
            }
        }
    }
    tiles
}

// ── Furniture instances ───────────────────────────────────────────────────────

/// Build renderable furniture instances. Day 4-7: position + zY only.
pub fn layout_to_furniture_instances(
    furniture: &[PlacedFurniture],
    catalog: &FurnitureCatalog,
) -> Vec<FurnitureInstance> {
    // Pre-compute desk zY per tile for surface-item z-sorting
    let mut desk_z: BTreeMap<(i32, i32), f32> = BTreeMap::new();
    for item in furniture {
        let Some(entry) = catalog.get(&item.type_id) else { continue };
        if !entry.is_desk { continue; }
        let dz = (item.row * TILE_SIZE + entry.sprite_height) as f32;
        for dr in 0..entry.footprint_h {
            for dc in 0..entry.footprint_w {
                let e = desk_z.entry((item.col + dc, item.row + dr)).or_insert(f32::NEG_INFINITY);
                if dz > *e { *e = dz; }
            }
        }
    }

    let mut instances = Vec::new();
    for item in furniture {
        let Some(entry) = catalog.get(&item.type_id) else { continue };

        let y = item.row * TILE_SIZE;
        let mut z_y = (y + entry.sprite_height) as f32;

        // Chair z-sort (matches TS layoutToFurnitureInstances)
        if entry.category == "chairs" {
            z_y = if entry.orientation.as_deref() == Some("back") {
                ((item.row + entry.footprint_h) * TILE_SIZE) as f32 + 1.0
            } else {
                ((item.row + 1) * TILE_SIZE) as f32
            };
        }

        // Surface items render in front of the desk they rest on
        if entry.can_place_on_surfaces {
            for dr in 0..entry.footprint_h {
                for dc in 0..entry.footprint_w {
                    if let Some(&dz) = desk_z.get(&(item.col + dc, item.row + dr)) {
                        if dz + 0.5 > z_y { z_y = dz + 0.5; }
                    }
                }
            }
        }

        // Wall items use bottom-row zY
        if entry.can_place_on_walls {
            z_y = ((item.row + 1) * TILE_SIZE) as f32;
        }

        instances.push(FurnitureInstance {
            uid: item.uid.clone(),
            type_id: item.type_id.clone(),
            col: item.col,
            row: item.row,
            z_y,
        });
    }
    instances
}

// ── Seats ─────────────────────────────────────────────────────────────────────

pub fn layout_to_seats(
    furniture: &[PlacedFurniture],
    catalog: &FurnitureCatalog,
) -> BTreeMap<String, Seat> {
    // Collect desk tiles for adjacent-desk facing detection
    let mut desk_tiles: BTreeSet<(i32, i32)> = BTreeSet::new();
    for item in furniture {
        let Some(entry) = catalog.get(&item.type_id) else { continue };
        if !entry.is_desk { continue; }
        for dr in 0..entry.footprint_h {
            for dc in 0..entry.footprint_w {
                desk_tiles.insert((item.col + dc, item.row + dr));
            }
        }
    }

    const ADJ: [(i32, i32, Direction); 4] = [
        (0, -1, Direction::Up),
        (0, 1, Direction::Down),
        (-1, 0, Direction::Left),
        (1, 0, Direction::Right),
    ];

    let mut seats = BTreeMap::new();

    for item in furniture {
        let Some(entry) = catalog.get(&item.type_id) else { continue };
        if entry.category != "chairs" { continue; }

        let bg = entry.background_tiles;
        let mut seat_count = 0i32;

        for dr in bg..entry.footprint_h {
            for dc in 0..entry.footprint_w {
                let tile_col = item.col + dc;
                let tile_row = item.row + dr;

                let facing_dir = if let Some(ref orient) = entry.orientation {
                    orientation_to_facing(orient)
                } else {
                    ADJ.iter()
                        .find(|(ddc, ddr, _)| desk_tiles.contains(&(tile_col + ddc, tile_row + ddr)))
                        .map(|(_, _, dir)| *dir)
                        .unwrap_or(Direction::Down)
                };

                let uid = if seat_count == 0 {
                    item.uid.clone()
                } else {
                    format!("{}:{}", item.uid, seat_count)
                };
                seats.insert(uid.clone(), Seat { uid, seat_col: tile_col, seat_row: tile_row, facing_dir, assigned: false });
                seat_count += 1;
            }
        }
    }

    seats
}

fn orientation_to_facing(orient: &str) -> Direction {
    match orient {
        "front" => Direction::Down,
        "back" => Direction::Up,
        "left" => Direction::Left,
        "right" | "side" => Direction::Right,
        _ => Direction::Down,
    }
}

// ── Layout deserialization ────────────────────────────────────────────────────

/// Parse an OfficeLayout from a JSON value (world.layout from HelloAck).
pub fn parse_layout(value: &Value) -> Option<OfficeLayout> {
    if value.is_null() { return None; }
    let version = value.get("version")?.as_u64()? as u32;
    if version != 1 { return None; }
    let cols = value.get("cols")?.as_i64()? as i32;
    let rows = value.get("rows")?.as_i64()? as i32;

    let tiles: Vec<TileType> = value
        .get("tiles")?
        .as_array()?
        .iter()
        .map(|v| TileType::from_u8(v.as_u64().unwrap_or(255) as u8))
        .collect();

    let furniture: Vec<PlacedFurniture> = value
        .get("furniture")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let uid = item.get("uid")?.as_str()?.to_string();
                    let type_id = item.get("type")?.as_str()?.to_string();
                    let col = item.get("col")?.as_i64()? as i32;
                    let row = item.get("row")?.as_i64()? as i32;
                    let color = parse_furniture_color(item.get("color"));
                    Some(PlacedFurniture { uid, type_id, col, row, color })
                })
                .collect()
        })
        .unwrap_or_default();

    let n = (cols * rows) as usize;
    let tile_colors: Vec<Option<TileColor>> = value
        .get("tileColors")
        .and_then(|tc| tc.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| {
                    if v.is_null() {
                        None
                    } else {
                        Some(TileColor {
                            h: v.get("h").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                            s: v.get("s").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                            b: v.get("b").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                            c: v.get("c").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                        })
                    }
                })
                .collect()
        })
        .unwrap_or_else(|| vec![None; n]);

    Some(OfficeLayout { version, cols, rows, tiles, furniture, tile_colors })
}

fn parse_furniture_color(value: Option<&Value>) -> Option<FurnitureColor> {
    let v = value?;
    if v.is_null() { return None; }
    Some(FurnitureColor {
        h: v.get("h").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        s: v.get("s").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        b: v.get("b").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        c: v.get("c").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        colorize: v.get("colorize").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}
