import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { AssetRegistry } from '../../src/assets/registry.js';

// Layout under test mirrors the bundled tree: an `assets/` root holding sibling
// `furniture/` and `characters/` dirs. `bundled` points at the furniture dir.
let root: string;
let furnitureDir: string;
let charsDir: string;

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-registry-test-'));
  furnitureDir = path.join(root, 'furniture');
  charsDir = path.join(root, 'characters');
  fs.mkdirSync(furnitureDir, { recursive: true });
  fs.mkdirSync(charsDir, { recursive: true });
});

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true });
});

describe('AssetRegistry character sprites (Day 18)', () => {
  it('serves char_N.png from the characters sibling of the bundled dir', () => {
    fs.writeFileSync(path.join(charsDir, 'char_0.png'), Buffer.from([0xde, 0xad]));
    fs.writeFileSync(path.join(charsDir, 'char_5.png'), Buffer.from([0xbe, 0xef]));

    const reg = new AssetRegistry({ bundled: furnitureDir, external: [] });

    expect(reg.getPng('char_0')).toEqual(Buffer.from([0xde, 0xad]));
    expect(reg.getPng('char_5')).toEqual(Buffer.from([0xbe, 0xef]));
  });

  it('returns null for an unregistered character id', () => {
    fs.writeFileSync(path.join(charsDir, 'char_0.png'), Buffer.from([0x01]));
    const reg = new AssetRegistry({ bundled: furnitureDir, external: [] });
    expect(reg.getPng('char_9')).toBeNull();
  });

  it('boots cleanly when the characters dir is absent', () => {
    fs.rmSync(charsDir, { recursive: true, force: true });
    const reg = new AssetRegistry({ bundled: furnitureDir, external: [] });
    expect(reg.getPng('char_0')).toBeNull();
  });

  it('ignores non-char_N png files in the characters dir', () => {
    fs.writeFileSync(path.join(charsDir, 'readme.png'), Buffer.from([0x00]));
    fs.writeFileSync(path.join(charsDir, 'char_2.png'), Buffer.from([0x42]));
    const reg = new AssetRegistry({ bundled: furnitureDir, external: [] });
    expect(reg.getPng('readme')).toBeNull();
    expect(reg.getPng('char_2')).toEqual(Buffer.from([0x42]));
  });
});
