import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { AgentsRegistry, type PersistedAgent } from '../../src/agents/registry.js';
import type { WriterTag } from '../../src/persistence/writerTag.js';

const OURS: WriterTag = { processId: 1, bootId: 'boot-test' };

let tmpDir: string;
let originalPath: string | undefined;

function makeAgent(id: number, sessionId: string): PersistedAgent {
  return {
    id,
    sessionId,
    palette: 0,
    hueShift: 0,
    lastSeenAt: 1_000_000,
  };
}

beforeEach(async () => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-reg-'));
  // Redirect the AGENTS_JSON_PATH for this test by mutating the env that the
  // paths.ts module resolves from. The module reads `os.homedir()` at import
  // time, so we monkey-patch process.env.HOME and reset module cache.
  originalPath = process.env.HOME;
  process.env.HOME = tmpDir;
  // Ensure the .pixel-agents subdir exists since AgentsRegistry writes through
  // writeTagged which expects to be able to mkdir the parent.
  vi.resetModules();
});

afterEach(() => {
  if (originalPath === undefined) delete process.env.HOME;
  else process.env.HOME = originalPath;
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

describe('AgentsRegistry', () => {
  it('starts empty when no file exists', async () => {
    const { AgentsRegistry: Reg } = await import('../../src/agents/registry.js');
    const reg = new Reg(OURS);
    expect(reg.cwds()).toEqual([]);
    expect(reg.forCwd('/anywhere')).toEqual([]);
  });

  it('upsert + remove + roundtrip across instances', async () => {
    const { AgentsRegistry: Reg } = await import('../../src/agents/registry.js');
    const reg1 = new Reg(OURS);
    reg1.upsert('/cwd-a', makeAgent(1, 'sess-1'));
    reg1.upsert('/cwd-a', makeAgent(2, 'sess-2'));
    reg1.upsert('/cwd-b', makeAgent(3, 'sess-3'));

    // Update existing
    reg1.upsert('/cwd-a', { ...makeAgent(1, 'sess-1'), palette: 4 });

    const reg2 = new Reg(OURS);
    const a = reg2.forCwd('/cwd-a');
    expect(a).toHaveLength(2);
    expect(a.find((x) => x.id === 1)?.palette).toBe(4);
    expect(reg2.forCwd('/cwd-b')).toHaveLength(1);
    expect(reg2.cwds().sort()).toEqual(['/cwd-a', '/cwd-b']);

    // Remove
    expect(reg2.remove('/cwd-a', 1)).toBe(true);
    expect(reg2.remove('/cwd-a', 999)).toBe(false);
    expect(reg2.forCwd('/cwd-a')).toHaveLength(1);

    // Removing the last agent in a cwd should also drop the cwd entry.
    reg2.remove('/cwd-b', 3);
    expect(reg2.cwds()).toEqual(['/cwd-a']);
  });

  it('setCwd replaces the list atomically', async () => {
    const { AgentsRegistry: Reg } = await import('../../src/agents/registry.js');
    const reg = new Reg(OURS);
    reg.setCwd('/work', [makeAgent(1, 's1'), makeAgent(2, 's2')]);
    expect(reg.forCwd('/work').map((a) => a.id)).toEqual([1, 2]);
    reg.setCwd('/work', [makeAgent(99, 's99')]);
    expect(reg.forCwd('/work').map((a) => a.id)).toEqual([99]);
  });

  it('tolerates a malformed file by starting empty', async () => {
    const { AGENTS_JSON_PATH } = await import('../../src/paths.js');
    fs.mkdirSync(path.dirname(AGENTS_JSON_PATH), { recursive: true });
    fs.writeFileSync(AGENTS_JSON_PATH, '{not json');
    const { AgentsRegistry: Reg } = await import('../../src/agents/registry.js');
    const reg = new Reg(OURS);
    expect(reg.cwds()).toEqual([]);
    // And subsequent writes still succeed.
    reg.upsert('/cwd', makeAgent(1, 's'));
    expect(reg.forCwd('/cwd')).toHaveLength(1);
  });

  it('tolerates an unknown schema version by starting empty', async () => {
    const { AGENTS_JSON_PATH } = await import('../../src/paths.js');
    fs.mkdirSync(path.dirname(AGENTS_JSON_PATH), { recursive: true });
    fs.writeFileSync(AGENTS_JSON_PATH, JSON.stringify({ version: 99, agents: { '/x': [] } }));
    const { AgentsRegistry: Reg } = await import('../../src/agents/registry.js');
    const reg = new Reg(OURS);
    expect(reg.cwds()).toEqual([]);
  });
});
