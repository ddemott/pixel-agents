#![allow(dead_code)]

use std::collections::BTreeMap;

use serde_json::Value;

// ── Catalog entry ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FurnitureCatalogEntry {
    pub id: String,
    pub name: String,
    pub category: String,
    pub footprint_w: i32,
    pub footprint_h: i32,
    pub is_desk: bool,
    pub can_place_on_walls: bool,
    pub can_place_on_surfaces: bool,
    pub background_tiles: i32,
    pub orientation: Option<String>,
    pub group_id: Option<String>,
    /// Pixel height of the sprite — used for zY calculation (Day 4-7 approximation).
    pub sprite_height: i32,
    /// "on" | "off" | None
    pub state: Option<String>,
}

// ── Catalog ───────────────────────────────────────────────────────────────────

pub struct FurnitureCatalog {
    pub entries: BTreeMap<String, FurnitureCatalogEntry>,
    /// groupId → ordered rotation cycle (sorted by ORIENTATION_ORDER)
    pub rotation_groups: BTreeMap<String, Vec<String>>,
    /// assetId → sibling state assetId (bidirectional: on↔off)
    pub state_groups: BTreeMap<String, String>,
}

const ORIENTATION_ORDER: &[&str] = &["front", "back", "side", "left", "right", "top"];

fn orientation_rank(o: &str) -> usize {
    ORIENTATION_ORDER.iter().position(|&x| x == o).unwrap_or(ORIENTATION_ORDER.len())
}

impl FurnitureCatalog {
    pub fn empty() -> Self {
        Self {
            entries: BTreeMap::new(),
            rotation_groups: BTreeMap::new(),
            state_groups: BTreeMap::new(),
        }
    }

    /// Build catalog from a WorldSnapshot `Value` (from HelloAck).
    pub fn from_wire(world: &Value) -> Self {
        let Some(arr) = world
            .get("assets")
            .and_then(|a| a.get("catalog"))
            .and_then(|c| c.as_array())
        else {
            return Self::empty();
        };

        let mut entries: BTreeMap<String, FurnitureCatalogEntry> = BTreeMap::new();

        for item in arr {
            let id = match item.get("id").and_then(|v| v.as_str()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            entries.insert(
                id.clone(),
                FurnitureCatalogEntry {
                    id,
                    name: item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    category: item.get("category").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    footprint_w: item.get("footprintW").and_then(|v| v.as_i64()).unwrap_or(1) as i32,
                    footprint_h: item.get("footprintH").and_then(|v| v.as_i64()).unwrap_or(1) as i32,
                    is_desk: item.get("isDesk").and_then(|v| v.as_bool()).unwrap_or(false),
                    can_place_on_walls: item.get("canPlaceOnWalls").and_then(|v| v.as_bool()).unwrap_or(false),
                    can_place_on_surfaces: item.get("canPlaceOnSurfaces").and_then(|v| v.as_bool()).unwrap_or(false),
                    background_tiles: item.get("backgroundTiles").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                    orientation: item.get("orientation").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    group_id: item.get("groupId").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    sprite_height: item.get("height").and_then(|v| v.as_i64()).unwrap_or(16) as i32,
                    state: item.get("state").and_then(|v| v.as_str()).map(|s| s.to_string()),
                },
            );
        }

        let (rotation_groups, state_groups) = build_groups(&entries);
        Self { entries, rotation_groups, state_groups }
    }

    pub fn get(&self, id: &str) -> Option<&FurnitureCatalogEntry> {
        self.entries.get(id)
    }

    pub fn get_on_state(&self, id: &str) -> Option<&str> {
        self.state_groups.get(id).map(|s| s.as_str())
    }
}

fn build_groups(
    entries: &BTreeMap<String, FurnitureCatalogEntry>,
) -> (BTreeMap<String, Vec<String>>, BTreeMap<String, String>) {
    // groupId → orientation → Vec<asset_id>
    let mut by_group_orient: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();

    for (id, entry) in entries {
        if let Some(ref gid) = entry.group_id {
            by_group_orient
                .entry(gid.clone())
                .or_default()
                .entry(entry.orientation.clone().unwrap_or_default())
                .or_default()
                .push(id.clone());
        }
    }

    let mut rotation_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut state_groups: BTreeMap<String, String> = BTreeMap::new();

    for (gid, by_orient) in &by_group_orient {
        let mut orient_keys: Vec<&String> = by_orient.keys().collect();

        // Rotation groups (multiple orientations, or single non-empty orientation)
        if orient_keys.len() > 1 || (orient_keys.len() == 1 && !orient_keys[0].is_empty()) {
            orient_keys.sort_by(|a, b| {
                orientation_rank(a).cmp(&orientation_rank(b)).then(a.cmp(b))
            });

            let cycle: Vec<String> = orient_keys
                .iter()
                .filter_map(|orient| {
                    let group = by_orient.get(*orient)?;
                    // Prefer non-"on" state as rotation representative
                    let rep = group
                        .iter()
                        .find(|id| entries.get(*id).and_then(|e| e.state.as_deref()) != Some("on"))
                        .or_else(|| group.first())?;
                    Some(rep.clone())
                })
                .collect();

            if cycle.len() > 1 {
                rotation_groups.insert(gid.clone(), cycle);
            }
        }

        // State groups: wire on↔off within each orientation bucket
        for group in by_orient.values() {
            let off = group.iter().find(|id| {
                entries.get(*id).and_then(|e| e.state.as_deref()) != Some("on")
            });
            let on = group.iter().find(|id| {
                entries.get(*id).and_then(|e| e.state.as_deref()) == Some("on")
            });
            if let (Some(off_id), Some(on_id)) = (off, on) {
                state_groups.insert(off_id.clone(), on_id.clone());
                state_groups.insert(on_id.clone(), off_id.clone());
            }
        }
    }

    (rotation_groups, state_groups)
}
