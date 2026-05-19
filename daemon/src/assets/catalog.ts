import * as fs from 'fs';
import * as path from 'path';

import {
  flattenManifest,
  type FurnitureAsset,
  type FurnitureManifest,
} from '../../../shared/assets/manifestUtils.js';

export type { FurnitureAsset };

export interface BuiltCatalog {
  assets: FurnitureAsset[];
  /** groupId → ordered list of asset IDs forming a rotation cycle */
  rotationGroups: Map<string, string[]>;
  /** assetId → asset ID of the toggled state sibling */
  stateGroups: Map<string, string>;
}

/**
 * Scan a furniture root directory (e.g. `webview-ui/public/assets/furniture`).
 * Each immediate subdirectory is expected to contain a `manifest.json`.
 * Subdirectories missing a manifest are silently skipped.
 */
export function scanFurnitureDir(furnitureDir: string): FurnitureAsset[] {
  if (!fs.existsSync(furnitureDir)) return [];

  const entries = fs.readdirSync(furnitureDir, { withFileTypes: true });
  const assets: FurnitureAsset[] = [];

  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const manifestPath = path.join(furnitureDir, entry.name, 'manifest.json');
    if (!fs.existsSync(manifestPath)) continue;

    let manifest: FurnitureManifest;
    try {
      manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as FurnitureManifest;
    } catch {
      continue; // malformed manifest — skip
    }

    const inherited = {
      groupId: manifest.id,
      name: manifest.name,
      category: manifest.category,
      canPlaceOnWalls: manifest.canPlaceOnWalls ?? false,
      canPlaceOnSurfaces: manifest.canPlaceOnSurfaces ?? false,
      backgroundTiles: manifest.backgroundTiles ?? 0,
    };

    if (manifest.type === 'asset') {
      // Single-asset furniture (no rotation / state groups)
      assets.push({
        id: manifest.id,
        name: manifest.name,
        label: manifest.name,
        category: manifest.category,
        file: manifest.file ?? `${manifest.id}.png`,
        width: manifest.width ?? 0,
        height: manifest.height ?? 0,
        footprintW: manifest.footprintW ?? 1,
        footprintH: manifest.footprintH ?? 1,
        isDesk: manifest.category === 'desks',
        canPlaceOnWalls: manifest.canPlaceOnWalls ?? false,
        canPlaceOnSurfaces: manifest.canPlaceOnSurfaces ?? false,
        backgroundTiles: manifest.backgroundTiles ?? 0,
      });
    } else if (manifest.type === 'group' && Array.isArray(manifest.members)) {
      for (const member of manifest.members) {
        assets.push(...flattenManifest(member, inherited));
      }
    }
  }

  return assets;
}

/**
 * Build rotation and state group maps from a flat asset list.
 *
 * Rotation groups: assets sharing a `groupId` with different `orientation`
 * values. Ordering follows the ORIENTATION_ORDER list so rotation cycles
 * are predictable regardless of manifest order.
 *
 * State groups: within a rotation group (same `groupId` + `orientation`),
 * assets with `state: 'on'` and `state: 'off'` form a toggle pair.
 * The map is bidirectional: on→off and off→on.
 */
export function buildGroups(assets: FurnitureAsset[]): BuiltCatalog {
  // groupId → orientation → asset[]
  const byGroupOrient = new Map<string, Map<string, FurnitureAsset[]>>();

  for (const asset of assets) {
    if (!asset.groupId) continue;
    let byOrient = byGroupOrient.get(asset.groupId);
    if (!byOrient) {
      byOrient = new Map();
      byGroupOrient.set(asset.groupId, byOrient);
    }
    const orient = asset.orientation ?? '';
    const list = byOrient.get(orient) ?? [];
    list.push(asset);
    byOrient.set(orient, list);
  }

  const rotationGroups = new Map<string, string[]>();
  const stateGroups = new Map<string, string>();

  for (const [groupId, byOrient] of byGroupOrient) {
    // Rotation: gather one representative per orientation (prefer state='off' or no state)
    const orientKeys = [...byOrient.keys()];
    if (orientKeys.length > 1 || (orientKeys.length === 1 && orientKeys[0] !== '')) {
      const cycle = orientKeys.sort(compareOrientations).map((orient) => {
        const group = byOrient.get(orient)!;
        // Pick off/unset state as the rotation representative
        const rep = group.find((a) => !a.state || a.state === 'off') ?? group[0]!;
        return rep.id;
      });
      if (cycle.length > 1) {
        rotationGroups.set(groupId, cycle);
      }
    }

    // State: within each orientation bucket, wire on↔off pairs
    for (const group of byOrient.values()) {
      const off = group.find((a) => !a.state || a.state === 'off');
      const on = group.find((a) => a.state === 'on');
      if (off && on) {
        stateGroups.set(off.id, on.id);
        stateGroups.set(on.id, off.id);
      }
    }
  }

  return { assets, rotationGroups, stateGroups };
}

const ORIENTATION_ORDER = ['front', 'back', 'side', 'left', 'right', 'top'];

function compareOrientations(a: string, b: string): number {
  const ai = ORIENTATION_ORDER.indexOf(a);
  const bi = ORIENTATION_ORDER.indexOf(b);
  if (ai === -1 && bi === -1) return a.localeCompare(b);
  if (ai === -1) return 1;
  if (bi === -1) return -1;
  return ai - bi;
}
