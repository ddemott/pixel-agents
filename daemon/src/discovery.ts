import * as fs from 'fs';
import * as path from 'path';

import { DAEMON_JSON_PATH, PIXEL_AGENTS_DIR } from './paths.js';

/**
 * Discovery file written to `~/.pixel-agents/daemon.json` so clients (TUI, future
 * GUI/web) and hook scripts can locate the running daemon. Multi-window safe:
 * clients pin to `bootId` and treat a mismatch as a dead connection.
 */
export interface DaemonDiscovery {
  /** UUIDv4 regenerated on every daemon boot. Clients pin to this. */
  bootId: string;
  /** Bearer token for client RPC authentication. */
  token: string;
  /** Daemon process ID. */
  pid: number;
  /** Unix domain socket path (Unix) or named pipe path (Windows). */
  socketPath: string;
  /** Optional TCP port for the hook HTTP server (set in later phases). */
  hookPort?: number;
  /** Timestamp the daemon started (ms since epoch). */
  startedAt: number;
  /** Daemon version string. */
  version: string;
}

function ensureDir(dir: string): void {
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  }
}

/** Read and parse daemon.json. Returns null if missing or malformed. */
export function readDiscovery(): DaemonDiscovery | null {
  try {
    if (!fs.existsSync(DAEMON_JSON_PATH)) return null;
    return JSON.parse(fs.readFileSync(DAEMON_JSON_PATH, 'utf-8')) as DaemonDiscovery;
  } catch {
    return null;
  }
}

/** Atomic write: tmp + rename, mode 0o600. */
export function writeDiscovery(discovery: DaemonDiscovery): void {
  ensureDir(path.dirname(DAEMON_JSON_PATH));
  const tmp = DAEMON_JSON_PATH + '.tmp';
  fs.writeFileSync(tmp, JSON.stringify(discovery, null, 2), { mode: 0o600 });
  fs.renameSync(tmp, DAEMON_JSON_PATH);
}

/** Delete daemon.json only if its PID matches ours (don't clobber another instance). */
export function clearDiscoveryIfOwned(pid: number): void {
  try {
    const existing = readDiscovery();
    if (existing && existing.pid === pid) {
      fs.unlinkSync(DAEMON_JSON_PATH);
    }
  } catch {
    // ignore
  }
}

/** Check if a PID is currently alive on the system. */
export function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    if (e instanceof Error && 'code' in e && (e as NodeJS.ErrnoException).code === 'EPERM') {
      // EPERM means the process exists but we can't signal it — still alive.
      return true;
    }
    return false;
  }
}

/** Ensure the runtime directory exists (called once at boot). */
export function ensurePixelAgentsDir(): void {
  ensureDir(PIXEL_AGENTS_DIR);
}
