import * as fs from 'fs';
import * as http from 'http';
import * as os from 'os';
import * as path from 'path';

import {
  DAEMON_JSON_NAME,
  HOOK_API_PREFIX,
  HOOK_TOKEN_ENV,
  HOOK_URL_ENV,
  SERVER_JSON_DIR,
  SERVER_JSON_NAME,
} from '../../../../constants.js';
import type { ServerConfig } from '../../../../httpServer.js';

/**
 * Resolved POST target for the hook script.
 * Populated by the chain: env override → daemon.json → server.json.
 */
interface HookTarget {
  hostname: string;
  port: number;
  path: string;
  token: string;
}

/** Subset of daemon.json fields used here (kept structurally compatible with
 *  daemon/src/discovery.ts:DaemonDiscovery without importing it). */
interface DaemonDiscoveryShape {
  token: string;
  hookPort?: number;
}

const DAEMON_JSON = path.join(os.homedir(), SERVER_JSON_DIR, DAEMON_JSON_NAME);
const SERVER_JSON = path.join(os.homedir(), SERVER_JSON_DIR, SERVER_JSON_NAME);

function readJson<T>(filePath: string): T | null {
  try {
    return JSON.parse(fs.readFileSync(filePath, 'utf-8')) as T;
  } catch {
    return null;
  }
}

/** Highest-priority discovery: PIXEL_AGENTS_HOOK_URL env var. */
function discoverFromEnv(): HookTarget | null {
  const url = process.env[HOOK_URL_ENV];
  if (!url) return null;
  try {
    const parsed = new URL(url);
    const portStr = parsed.port;
    const port = portStr ? Number(portStr) : parsed.protocol === 'https:' ? 443 : 80;
    if (!Number.isFinite(port) || port <= 0) return null;
    const hookPath =
      parsed.pathname && parsed.pathname !== '/' ? parsed.pathname : `${HOOK_API_PREFIX}/claude`;
    return {
      hostname: parsed.hostname || '127.0.0.1',
      port,
      path: hookPath,
      token: process.env[HOOK_TOKEN_ENV] ?? '',
    };
  } catch {
    return null;
  }
}

/** Daemon discovery: only counts when daemon is exposing a hook HTTP port. */
function discoverFromDaemon(): HookTarget | null {
  const cfg = readJson<DaemonDiscoveryShape>(DAEMON_JSON);
  if (!cfg || typeof cfg.hookPort !== 'number' || typeof cfg.token !== 'string') return null;
  return {
    hostname: '127.0.0.1',
    port: cfg.hookPort,
    path: `${HOOK_API_PREFIX}/claude`,
    token: cfg.token,
  };
}

/**
 * Legacy: VS Code extension's PixelAgentsServer publishes server.json.
 *
 * TRANSITIONAL (debt review): now that the daemon owns the hook server, this
 * branch is only reachable when the extension runs *without* a daemon. Keep it
 * until the extension-hosted server is formally deprecated (Phase 6); the
 * env → daemon.json → server.json order means a live daemon always wins, so
 * this never shadows the daemon path.
 */
function discoverFromServer(): HookTarget | null {
  const cfg = readJson<ServerConfig>(SERVER_JSON);
  if (!cfg || typeof cfg.port !== 'number' || typeof cfg.token !== 'string') return null;
  return {
    hostname: '127.0.0.1',
    port: cfg.port,
    path: `${HOOK_API_PREFIX}/claude`,
    token: cfg.token,
  };
}

function discoverTarget(): HookTarget | null {
  return discoverFromEnv() ?? discoverFromDaemon() ?? discoverFromServer();
}

async function main(): Promise<void> {
  let input = '';
  for await (const chunk of process.stdin) input += chunk;

  let data: Record<string, unknown>;
  try {
    data = JSON.parse(input);
  } catch {
    process.exit(0);
  }

  const target = discoverTarget();
  if (!target) process.exit(0);

  const body = JSON.stringify(data);
  return new Promise((resolve) => {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      'Content-Length': String(Buffer.byteLength(body)),
    };
    if (target.token) headers.Authorization = `Bearer ${target.token}`;

    const req = http.request(
      {
        hostname: target.hostname,
        port: target.port,
        path: target.path,
        method: 'POST',
        headers,
        timeout: 2000,
      },
      () => resolve(),
    );
    req.on('error', () => resolve());
    req.on('timeout', () => {
      req.destroy();
      resolve();
    });
    req.end(body);
  });
}

main()
  .catch(() => {})
  .finally(() => process.exit(0));
