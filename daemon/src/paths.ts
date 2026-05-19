import * as os from 'os';
import * as path from 'path';

/** Shared root directory for all Pixel Agents runtime state. */
export const PIXEL_AGENTS_DIR = path.join(os.homedir(), '.pixel-agents');

/** Daemon discovery file (port, PID, auth token, bootId, socket path). */
export const DAEMON_JSON_PATH = path.join(PIXEL_AGENTS_DIR, 'daemon.json');

/** Daemon Unix domain socket. */
export const DAEMON_SOCKET_PATH = path.join(PIXEL_AGENTS_DIR, 'daemon.sock');

/** Daemon log directory. */
export const DAEMON_LOG_DIR = path.join(PIXEL_AGENTS_DIR, 'logs');

/** User-level config file (shared with the VS Code extension). */
export const CONFIG_JSON_PATH = path.join(PIXEL_AGENTS_DIR, 'config.json');

/** Persisted agent registry (per-cwd index of live + restorable sessions). Written by daemon, read on restart. */
export const AGENTS_JSON_PATH = path.join(PIXEL_AGENTS_DIR, 'agents.json');
