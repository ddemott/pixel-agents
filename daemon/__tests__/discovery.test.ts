import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

/**
 * WHO: daemon boot discovery layer (src/discovery.ts).
 * WHAT: write/read of ~/.pixel-agents/daemon.json + PID liveness + ownership-
 *       guarded cleanup — the contract clients & hook scripts use to find us.
 * WHY: boot logic had no unit tests (smoke-only since Phase 1). These pin the
 *      discovery contract (valid bootId/token/PID round-trips; cleanup only
 *      removes our own file) so a refactor can't silently break daemon launch.
 * HOW: PIXEL_AGENTS_DIR is resolved at import time from PIXEL_AGENTS_HOME, so
 *      each test sets a fresh tmp home, resets the module registry, and
 *      dynamic-imports a clean copy of discovery + paths.
 */

let tmpHome: string;
let originalHome: string | undefined;

async function freshDiscovery() {
  const { vi } = await import('vitest');
  vi.resetModules();
  const discovery = await import('../src/discovery.js');
  const paths = await import('../src/paths.js');
  return { discovery, paths };
}

function sampleDiscovery() {
  return {
    bootId: '11111111-2222-3333-4444-555555555555',
    token: 'a'.repeat(64),
    pid: process.pid,
    socketPath: '/tmp/x.sock',
    startedAt: 1_700_000_000_000,
    version: '0.0.1',
    hookPort: 41234,
  };
}

beforeEach(() => {
  tmpHome = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-disco-'));
  originalHome = process.env.PIXEL_AGENTS_HOME;
  process.env.PIXEL_AGENTS_HOME = tmpHome;
});

afterEach(() => {
  if (originalHome === undefined) delete process.env.PIXEL_AGENTS_HOME;
  else process.env.PIXEL_AGENTS_HOME = originalHome;
  try {
    fs.rmSync(tmpHome, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

describe('daemon discovery', () => {
  it('round-trips all fields through write → read', async () => {
    const { discovery } = await freshDiscovery();
    const d = sampleDiscovery();
    discovery.writeDiscovery(d);
    expect(discovery.readDiscovery()).toEqual(d);
  });

  it('writes daemon.json into PIXEL_AGENTS_HOME with 0o600', async () => {
    const { discovery, paths } = await freshDiscovery();
    discovery.writeDiscovery(sampleDiscovery());
    expect(paths.DAEMON_JSON_PATH.startsWith(tmpHome)).toBe(true);
    expect(fs.existsSync(paths.DAEMON_JSON_PATH)).toBe(true);
    if (process.platform !== 'win32') {
      expect(fs.statSync(paths.DAEMON_JSON_PATH).mode & 0o777).toBe(0o600);
    }
  });

  it('returns null when daemon.json is absent', async () => {
    const { discovery } = await freshDiscovery();
    expect(discovery.readDiscovery()).toBeNull();
  });

  it('returns null on malformed daemon.json', async () => {
    const { discovery, paths } = await freshDiscovery();
    fs.mkdirSync(path.dirname(paths.DAEMON_JSON_PATH), { recursive: true });
    fs.writeFileSync(paths.DAEMON_JSON_PATH, 'not json!!!');
    expect(discovery.readDiscovery()).toBeNull();
  });

  it('isProcessAlive: true for self, false for an unused PID', async () => {
    const { discovery } = await freshDiscovery();
    expect(discovery.isProcessAlive(process.pid)).toBe(true);
    // A very high PID is overwhelmingly unlikely to exist.
    expect(discovery.isProcessAlive(2_147_400_000)).toBe(false);
  });

  it('clearDiscoveryIfOwned removes our own file but spares another PID', async () => {
    const { discovery, paths } = await freshDiscovery();

    // Owned by us → removed.
    discovery.writeDiscovery({ ...sampleDiscovery(), pid: process.pid });
    discovery.clearDiscoveryIfOwned(process.pid);
    expect(fs.existsSync(paths.DAEMON_JSON_PATH)).toBe(false);

    // Owned by a different live-looking PID → left intact.
    discovery.writeDiscovery({ ...sampleDiscovery(), pid: process.pid });
    discovery.clearDiscoveryIfOwned(999_999); // not our pid
    expect(fs.existsSync(paths.DAEMON_JSON_PATH)).toBe(true);
  });

  it('ensurePixelAgentsDir (re)creates the runtime directory', async () => {
    const { discovery, paths } = await freshDiscovery();
    // tmpHome (== PIXEL_AGENTS_DIR) is created by mkdtempSync; remove it to
    // prove ensurePixelAgentsDir builds it from nothing.
    fs.rmSync(paths.PIXEL_AGENTS_DIR, { recursive: true, force: true });
    expect(fs.existsSync(paths.PIXEL_AGENTS_DIR)).toBe(false);
    discovery.ensurePixelAgentsDir();
    expect(fs.existsSync(paths.PIXEL_AGENTS_DIR)).toBe(true);
  });
});
