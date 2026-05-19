import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { buildGroups, scanFurnitureDir } from '../../src/assets/catalog.js';

// ── helpers ────────────────────────────────────────────────────────────────

let tmpDir: string;

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-catalog-test-'));
});

afterEach(() => {
  fs.rmSync(tmpDir, { recursive: true, force: true });
});

function writeManifest(name: string, manifest: object): void {
  const dir = path.join(tmpDir, name);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'manifest.json'), JSON.stringify(manifest));
}

// ── scanFurnitureDir ───────────────────────────────────────────────────────

describe('scanFurnitureDir', () => {
  it('returns empty for non-existent dir', () => {
    expect(scanFurnitureDir('/no/such/dir')).toEqual([]);
  });

  it('skips subdirectory without manifest.json', () => {
    fs.mkdirSync(path.join(tmpDir, 'NAKED'), { recursive: true });
    expect(scanFurnitureDir(tmpDir)).toEqual([]);
  });

  it('skips malformed manifest', () => {
    const dir = path.join(tmpDir, 'BAD');
    fs.mkdirSync(dir);
    fs.writeFileSync(path.join(dir, 'manifest.json'), '{ invalid json ');
    expect(scanFurnitureDir(tmpDir)).toEqual([]);
  });

  it('loads a single-asset manifest', () => {
    writeManifest('CACTUS', {
      id: 'CACTUS',
      name: 'Cactus',
      category: 'decor',
      type: 'asset',
      file: 'CACTUS.png',
      width: 16,
      height: 32,
      footprintW: 1,
      footprintH: 1,
      canPlaceOnWalls: false,
      canPlaceOnSurfaces: false,
      backgroundTiles: 0,
    });
    const assets = scanFurnitureDir(tmpDir);
    expect(assets).toHaveLength(1);
    expect(assets[0]).toMatchObject({
      id: 'CACTUS',
      name: 'Cactus',
      category: 'decor',
      file: 'CACTUS.png',
    });
  });

  it('flattens a rotation-group manifest', () => {
    writeManifest('DESK', {
      id: 'DESK',
      name: 'Desk',
      category: 'desks',
      type: 'group',
      groupType: 'rotation',
      rotationScheme: '2-way',
      canPlaceOnWalls: false,
      canPlaceOnSurfaces: false,
      backgroundTiles: 1,
      members: [
        {
          type: 'asset',
          id: 'DESK_FRONT',
          file: 'DESK_FRONT.png',
          width: 48,
          height: 32,
          footprintW: 3,
          footprintH: 2,
          orientation: 'front',
        },
        {
          type: 'asset',
          id: 'DESK_SIDE',
          file: 'DESK_SIDE.png',
          width: 16,
          height: 64,
          footprintW: 1,
          footprintH: 4,
          orientation: 'side',
        },
      ],
    });
    const assets = scanFurnitureDir(tmpDir);
    expect(assets).toHaveLength(2);
    expect(assets.map((a) => a.id)).toEqual(expect.arrayContaining(['DESK_FRONT', 'DESK_SIDE']));
    expect(assets[0]!.groupId).toBe('DESK');
    expect(assets[0]!.isDesk).toBe(true);
  });

  it('merges assets from multiple subdirs', () => {
    writeManifest('CACTUS', {
      id: 'CACTUS',
      name: 'Cactus',
      category: 'decor',
      type: 'asset',
      file: 'CACTUS.png',
      width: 16,
      height: 32,
      footprintW: 1,
      footprintH: 1,
      canPlaceOnWalls: false,
      canPlaceOnSurfaces: false,
      backgroundTiles: 0,
    });
    writeManifest('CLOCK', {
      id: 'CLOCK',
      name: 'Clock',
      category: 'wall',
      type: 'asset',
      file: 'CLOCK.png',
      width: 16,
      height: 16,
      footprintW: 1,
      footprintH: 1,
      canPlaceOnWalls: true,
      canPlaceOnSurfaces: false,
      backgroundTiles: 0,
    });
    const assets = scanFurnitureDir(tmpDir);
    expect(assets).toHaveLength(2);
  });
});

// ── buildGroups ────────────────────────────────────────────────────────────

describe('buildGroups', () => {
  it('empty input', () => {
    const { assets, rotationGroups, stateGroups } = buildGroups([]);
    expect(assets).toHaveLength(0);
    expect(rotationGroups.size).toBe(0);
    expect(stateGroups.size).toBe(0);
  });

  it('builds rotation group from shared groupId', () => {
    const assets = [
      {
        id: 'DESK_FRONT',
        name: 'Desk',
        label: 'Desk',
        category: 'desks',
        file: 'f',
        width: 48,
        height: 32,
        footprintW: 3,
        footprintH: 2,
        isDesk: true,
        canPlaceOnWalls: false,
        groupId: 'DESK',
        orientation: 'front',
      },
      {
        id: 'DESK_SIDE',
        name: 'Desk',
        label: 'Desk',
        category: 'desks',
        file: 's',
        width: 16,
        height: 64,
        footprintW: 1,
        footprintH: 4,
        isDesk: true,
        canPlaceOnWalls: false,
        groupId: 'DESK',
        orientation: 'side',
      },
    ];
    const { rotationGroups } = buildGroups(assets);
    const cycle = rotationGroups.get('DESK');
    expect(cycle).toBeDefined();
    expect(cycle).toContain('DESK_FRONT');
    expect(cycle).toContain('DESK_SIDE');
  });

  it('rotation cycle is sorted front-before-side', () => {
    const assets = [
      {
        id: 'DESK_SIDE',
        name: 'Desk',
        label: 'Desk',
        category: 'desks',
        file: 's',
        width: 16,
        height: 64,
        footprintW: 1,
        footprintH: 4,
        isDesk: true,
        canPlaceOnWalls: false,
        groupId: 'DESK',
        orientation: 'side',
      },
      {
        id: 'DESK_FRONT',
        name: 'Desk',
        label: 'Desk',
        category: 'desks',
        file: 'f',
        width: 48,
        height: 32,
        footprintW: 3,
        footprintH: 2,
        isDesk: true,
        canPlaceOnWalls: false,
        groupId: 'DESK',
        orientation: 'front',
      },
    ];
    const { rotationGroups } = buildGroups(assets);
    const cycle = rotationGroups.get('DESK')!;
    expect(cycle[0]).toBe('DESK_FRONT');
    expect(cycle[1]).toBe('DESK_SIDE');
  });

  it('builds bidirectional state group', () => {
    const assets = [
      {
        id: 'MONITOR_OFF',
        name: 'Monitor',
        label: 'Monitor',
        category: 'electronics',
        file: 'off',
        width: 16,
        height: 16,
        footprintW: 1,
        footprintH: 1,
        isDesk: false,
        canPlaceOnWalls: false,
        groupId: 'MONITOR',
        orientation: 'front',
        state: 'off',
      },
      {
        id: 'MONITOR_ON',
        name: 'Monitor',
        label: 'Monitor',
        category: 'electronics',
        file: 'on',
        width: 16,
        height: 16,
        footprintW: 1,
        footprintH: 1,
        isDesk: false,
        canPlaceOnWalls: false,
        groupId: 'MONITOR',
        orientation: 'front',
        state: 'on',
      },
    ];
    const { stateGroups } = buildGroups(assets);
    expect(stateGroups.get('MONITOR_OFF')).toBe('MONITOR_ON');
    expect(stateGroups.get('MONITOR_ON')).toBe('MONITOR_OFF');
  });

  it('single-asset group has no rotation entry', () => {
    const assets = [
      {
        id: 'CACTUS',
        name: 'Cactus',
        label: 'Cactus',
        category: 'decor',
        file: 'c',
        width: 16,
        height: 32,
        footprintW: 1,
        footprintH: 1,
        isDesk: false,
        canPlaceOnWalls: false,
      },
    ];
    const { rotationGroups } = buildGroups(assets);
    expect(rotationGroups.size).toBe(0);
  });
});
