import * as os from 'os';
import * as path from 'path';

/**
 * Shared root directory for all Pixel Agents runtime state.
 *
 * Set `PIXEL_AGENTS_HOME` to point the entire on-disk layout (daemon.json,
 * socket, logs, config, layout, agents.json, ...) at an alternate directory.
 * Used by the hook integration test to isolate from the developer's real
 * daemon. Resolved once at process start — children forked later in the
 * same process will inherit, but a re-read of the env var won't change it.
 */
export const PIXEL_AGENTS_DIR =
  process.env.PIXEL_AGENTS_HOME ?? path.join(os.homedir(), '.pixel-agents');

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

/** User-level layout file (shared with the VS Code extension). */
export const LAYOUT_JSON_PATH = path.join(PIXEL_AGENTS_DIR, 'layout.json');

/** Daemon-private state scratchpad (anything that doesn't belong in the typed registries). */
export const DAEMON_STATE_PATH = path.join(PIXEL_AGENTS_DIR, 'daemon-state.json');

/**
 * Bundled furniture asset directory.
 *
 * Set `PIXEL_AGENTS_ASSETS_DIR` to redirect all bundled-asset reads (e.g.
 * for test isolation). Defaults to `webview-ui/public/assets/furniture`
 * relative to the project root (4 levels up from the compiled daemon entry).
 */
export function getBundledAssetsDir(): string {
  if (process.env.PIXEL_AGENTS_ASSETS_DIR) {
    return path.join(process.env.PIXEL_AGENTS_ASSETS_DIR, 'furniture');
  }
  // Compiled daemon lives at daemon/dist/daemon/src/server.js → 4 levels up = project root.
  const projectRoot = path.resolve(new URL('../../../..', import.meta.url).pathname);
  return path.join(projectRoot, 'webview-ui', 'public', 'assets', 'furniture');
}
