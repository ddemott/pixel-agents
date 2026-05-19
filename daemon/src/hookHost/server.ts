import * as crypto from 'crypto';
import * as http from 'http';

import type { Logger } from '../logging/logger.js';

const HOOK_API_PREFIX = '/api/hooks';
const MAX_HOOK_BODY_SIZE = 65_536;
const REQUEST_TIMEOUT_MS = 5_000;

/** Hook payload routed to the bridge after auth + body parse succeeds. */
export type HookEventCallback = (providerId: string, event: Record<string, unknown>) => void;

export interface HookHTTPServerOptions {
  /** Daemon's RPC auth token — reused as the Bearer for hook POSTs. */
  token: string;
  /** Routed event sink. Called once per validated POST. */
  onEvent: HookEventCallback;
  /** Structured logger (NDJSON). */
  logger: Logger;
}

export interface HookHTTPServerHandle {
  /** Bound port. Published in daemon.json as `hookPort`. */
  readonly port: number;
  /** Stop accepting new connections and close the underlying server. */
  close(): Promise<void>;
}

/**
 * Daemon-side hook HTTP server. Listens on 127.0.0.1:0, accepts
 * `POST /api/hooks/:providerId` with a `Bearer ${daemon.token}` header, and
 * forwards the parsed JSON body to `onEvent`.
 *
 * This replaces the extension's `PixelAgentsServer` for daemon-hosted runs:
 * the hook script discovers us via `daemon.json` (its `hookPort` field) and
 * authenticates with the same token already used for UDS RPC. There is no
 * `server.json` write — `daemon.json` is the only discovery surface when the
 * daemon owns the hook server.
 */
export async function startHookServer(opts: HookHTTPServerOptions): Promise<HookHTTPServerHandle> {
  const expectedAuth = `Bearer ${opts.token}`;
  const expectedBuf = Buffer.from(expectedAuth);

  const server = http.createServer((req, res) => {
    handleRequest(req, res, expectedBuf, opts);
  });
  server.setTimeout(REQUEST_TIMEOUT_MS);

  await new Promise<void>((resolve, reject) => {
    const onError = (err: Error): void => {
      server.removeListener('listening', onListening);
      reject(err);
    };
    const onListening = (): void => {
      server.removeListener('error', onError);
      resolve();
    };
    server.once('error', onError);
    server.once('listening', onListening);
    // 127.0.0.1 only — no external accessor, ever.
    server.listen(0, '127.0.0.1');
  });

  server.on('error', (err) => {
    opts.logger.error({ module: 'hookServer', error: err.message }, 'http server error');
  });

  const addr = server.address();
  if (!addr || typeof addr !== 'object') {
    server.close();
    throw new Error('hook server bound but no address returned');
  }
  const port = addr.port;

  opts.logger.info({ module: 'hookServer', port }, 'hook http server listening');

  return {
    port,
    close: () =>
      new Promise<void>((resolve) => {
        server.close(() => resolve());
      }),
  };
}

function handleRequest(
  req: http.IncomingMessage,
  res: http.ServerResponse,
  expectedBuf: Buffer,
  opts: HookHTTPServerOptions,
): void {
  const url = req.url ?? '';

  if (req.method === 'GET' && url === '/api/health') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ status: 'ok', pid: process.pid }));
    return;
  }

  if (req.method !== 'POST' || !url.startsWith(HOOK_API_PREFIX + '/')) {
    res.writeHead(404);
    res.end();
    return;
  }

  // Timing-safe Bearer compare so length/contents leak no side-channel.
  const authHeader = req.headers['authorization'] ?? '';
  const authBuf = Buffer.from(authHeader);
  if (authBuf.length !== expectedBuf.length || !crypto.timingSafeEqual(authBuf, expectedBuf)) {
    res.writeHead(401);
    res.end('unauthorized');
    return;
  }

  const providerId = url.slice(HOOK_API_PREFIX.length + 1);
  if (!providerId || !/^[a-z0-9-]+$/.test(providerId)) {
    res.writeHead(400);
    res.end('invalid provider id');
    return;
  }

  let body = '';
  let bodySize = 0;
  let responded = false;

  req.on('data', (chunk: Buffer) => {
    bodySize += chunk.length;
    if (bodySize > MAX_HOOK_BODY_SIZE && !responded) {
      responded = true;
      res.writeHead(413);
      res.end('payload too large');
      req.destroy();
      return;
    }
    if (!responded) body += chunk.toString('utf-8');
  });

  req.on('end', () => {
    if (responded) return;
    let event: Record<string, unknown>;
    try {
      event = JSON.parse(body) as Record<string, unknown>;
    } catch {
      res.writeHead(400);
      res.end('invalid json');
      return;
    }
    if (!event.session_id || !event.hook_event_name) {
      res.writeHead(400);
      res.end('missing required fields');
      return;
    }
    try {
      opts.onEvent(providerId, event);
    } catch (e) {
      opts.logger.error(
        { module: 'hookServer', providerId, error: e instanceof Error ? e.message : String(e) },
        'event callback threw',
      );
    }
    res.writeHead(200);
    res.end('ok');
  });
}
