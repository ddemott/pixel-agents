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
