import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import {
  isOwnWrite,
  readTagged,
  type WriterTag,
  writeTagged,
} from '../../src/persistence/writerTag.js';

let tmpDir: string;
let file: string;

const OURS: WriterTag = { processId: 100, bootId: 'boot-A' };
const THEIRS: WriterTag = { processId: 999, bootId: 'boot-B' };

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-wtag-'));
  file = path.join(tmpDir, 'data.json');
});

afterEach(() => {
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

describe('writerTag', () => {
  it('writes and reads a tagged file with the payload intact', () => {
    writeTagged(file, { name: 'pixel', n: 7 }, OURS);
    const result = readTagged<{ name: string; n: number }>(file);
    expect(result).not.toBeNull();
    if (!result) return;
    expect(result.tag).toEqual(OURS);
    expect(result.data).toEqual({ name: 'pixel', n: 7 });
    // _writer must be stripped from `data`.
    expect((result.data as Record<string, unknown>)._writer).toBeUndefined();
  });

  it('isOwnWrite matches by bootId, not processId', () => {
    expect(isOwnWrite(OURS, OURS)).toBe(true);
    expect(isOwnWrite(THEIRS, OURS)).toBe(false);
    expect(isOwnWrite({ processId: 42, bootId: OURS.bootId }, OURS)).toBe(true);
    expect(isOwnWrite(null, OURS)).toBe(false);
  });

  it('returns null for a missing file', () => {
    expect(readTagged(file)).toBeNull();
  });

  it('returns null for malformed JSON without throwing', () => {
    fs.writeFileSync(file, '{not json');
    expect(readTagged(file)).toBeNull();
  });

  it('handles an untagged but valid JSON file', () => {
    fs.writeFileSync(file, JSON.stringify({ a: 1 }));
    const result = readTagged<{ a: number }>(file);
    expect(result).not.toBeNull();
    if (!result) return;
    expect(result.tag).toBeNull();
    expect(result.data).toEqual({ a: 1 });
  });

  it('writes atomically (no .tmp file leaked on success)', () => {
    writeTagged(file, { x: 1 }, OURS);
    expect(fs.existsSync(file)).toBe(true);
    expect(fs.existsSync(file + '.tmp')).toBe(false);
  });
});
