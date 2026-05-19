import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { FileStateStore } from '../../src/agents/fileStateStore.js';

let tmpDir: string;
let storePath: string;

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-state-'));
  storePath = path.join(tmpDir, 'agents.json');
});

afterEach(() => {
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

describe('FileStateStore', () => {
  it('returns undefined for unknown keys on a fresh store', () => {
    const store = new FileStateStore(storePath);
    expect(store.get('anything')).toBeUndefined();
  });

  it('roundtrips simple values through disk', () => {
    const store1 = new FileStateStore(storePath);
    store1.set('agents', [{ id: 1, sessionId: 'a' }]);
    store1.set('nextAgentId', 2);

    // New instance reads from disk.
    const store2 = new FileStateStore(storePath);
    expect(store2.get<{ id: number; sessionId: string }[]>('agents')).toEqual([
      { id: 1, sessionId: 'a' },
    ]);
    expect(store2.get<number>('nextAgentId')).toBe(2);
  });

  it('delete via set(undefined) removes the key', () => {
    const store = new FileStateStore(storePath);
    store.set('temp', 'hello');
    expect(store.get('temp')).toBe('hello');
    store.set('temp', undefined);
    expect(store.get('temp')).toBeUndefined();
    // And reload confirms the disk write
    const reload = new FileStateStore(storePath);
    expect(reload.get('temp')).toBeUndefined();
  });

  it('writes atomically (no .tmp left behind on success)', () => {
    const store = new FileStateStore(storePath);
    store.set('x', 1);
    expect(fs.existsSync(storePath)).toBe(true);
    expect(fs.existsSync(storePath + '.tmp')).toBe(false);
  });

  it('tolerates a malformed file by starting empty', () => {
    fs.writeFileSync(storePath, '{not valid json');
    const store = new FileStateStore(storePath);
    expect(store.get('whatever')).toBeUndefined();
    // Subsequent writes still succeed.
    store.set('ok', 1);
    expect(store.get('ok')).toBe(1);
  });
});
