import * as crypto from 'crypto';
import type { Socket } from 'net';

import type { ConnectionScope, DispatchContext, MethodRegistry } from './dispatch.js';
import { encodeNdjson, type Frame, FrameDecoder, FramingError } from './framing.js';
import {
  type Hello,
  type HelloAck,
  isHello,
  isReq,
  PROTO_VERSION,
  type Res,
  type WorldSnapshot,
} from './wire.js';

const HELLO_TIMEOUT_MS = 5_000;

export interface ConnectionContext {
  /** Bearer token stored in `~/.pixel-agents/daemon.json`. */
  expectedToken: string;
  bootId: string;
  daemonVersion: string;
  /** Build an initial WorldSnapshot at handshake time. */
  buildWorldSnapshot: () => WorldSnapshot;
  /** Build a fresh per-connection sessionId. */
  newSessionId?: () => string;
  /**
   * Method registry shared by every connection. Day 7-8 introduces this; Day
   * 3-4 connections returned `method_not_implemented` for every request.
   */
  registry: MethodRegistry;
  /** Daemon-wide handles passed to every handler. */
  dispatchContext: DispatchContext;
  /**
   * Notified after a successful hello so the caller can register the per-conn
   * scope with the broadcast bus (Day 5+).
   */
  onAuthenticated?: (sock: Socket, scope: ConnectionScope) => void;
}

interface ConnectionState {
  authed: boolean;
  scope: ConnectionScope;
}

/**
 * Wire a freshly-accepted socket into the RPC pipeline.
 *
 * First NDJSON message must be `hello` with a valid token; otherwise we close.
 * Binary frames (0x01/0x02/0x03) before auth are also a close-the-socket
 * condition.
 */
export function attachConnection(sock: Socket, ctx: ConnectionContext): void {
  const decoder = new FrameDecoder();
  const state: ConnectionState = {
    authed: false,
    scope: {
      sessionId: (ctx.newSessionId ?? defaultSessionId)(),
      subscriptions: new Set<string>(),
      sock,
    },
  };

  const helloTimer = setTimeout(() => {
    if (!state.authed) {
      closeWithError(sock, 'hello_timeout', `no hello within ${HELLO_TIMEOUT_MS} ms`);
    }
  }, HELLO_TIMEOUT_MS);
  helloTimer.unref?.();

  sock.on('data', (chunk: Buffer) => {
    try {
      decoder.push(chunk);
    } catch (e) {
      const msg = e instanceof FramingError ? e.message : String(e);
      closeWithError(sock, 'framing_error', msg);
      return;
    }
    for (const frame of decoder.drain()) {
      void handleFrame(sock, state, ctx, frame, helloTimer);
      if (sock.destroyed) return;
    }
  });

  sock.on('close', () => {
    clearTimeout(helloTimer);
  });

  sock.on('error', () => {
    // Node will fire 'close' after 'error'; cleanup handled there.
  });
}

async function handleFrame(
  sock: Socket,
  state: ConnectionState,
  ctx: ConnectionContext,
  frame: Frame,
  helloTimer: NodeJS.Timeout,
): Promise<void> {
  // Binary frames before authentication are forbidden.
  if (!state.authed && frame.kind !== 'ndjson') {
    closeWithError(sock, 'unauthenticated', `binary frame ${frame.kind} before hello`);
    return;
  }

  if (frame.kind !== 'ndjson') {
    // Post-auth binary frames are valid wire format but no consumer is wired
    // yet — PTY in/out and asset blobs land in later phases.
    return;
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(frame.line);
  } catch {
    closeWithError(sock, 'bad_json', 'NDJSON line is not valid JSON');
    return;
  }

  if (!state.authed) {
    if (!isHello(parsed)) {
      closeWithError(sock, 'expected_hello', 'first message must be `hello`');
      return;
    }
    handleHello(sock, state, ctx, parsed, helloTimer);
    return;
  }

  if (isReq(parsed)) {
    const res = await ctx.registry.dispatch(
      parsed.reqId,
      parsed.method,
      parsed.params,
      state.scope,
      ctx.dispatchContext,
    );
    sendRes(sock, res);
    return;
  }

  // Unknown / out-of-spec message: ignore for now. Later phases may upgrade
  // to a close-the-socket condition.
}

function handleHello(
  sock: Socket,
  state: ConnectionState,
  ctx: ConnectionContext,
  hello: Hello,
  helloTimer: NodeJS.Timeout,
): void {
  if (hello.protoVersion !== PROTO_VERSION) {
    closeWithError(
      sock,
      'proto_mismatch',
      `client protoVersion=${hello.protoVersion}, daemon=${PROTO_VERSION}`,
    );
    return;
  }
  if (!verifyToken(hello.token, ctx.expectedToken)) {
    closeWithError(sock, 'bad_token', 'auth token did not match');
    return;
  }

  clearTimeout(helloTimer);
  state.authed = true;

  const ack: HelloAck = {
    kind: 'helloAck',
    daemonVersion: ctx.daemonVersion,
    protoVersion: PROTO_VERSION,
    bootId: ctx.bootId,
    sessionId: state.scope.sessionId,
    subscriptions: [],
    world: ctx.buildWorldSnapshot(),
  };
  sock.write(encodeNdjson(ack));
  ctx.onAuthenticated?.(sock, state.scope);
}

function sendRes(sock: Socket, res: Res): void {
  if (sock.destroyed || !sock.writable) return;
  try {
    sock.write(encodeNdjson(res));
  } catch {
    sock.destroy();
  }
}

function closeWithError(sock: Socket, code: string, message: string): void {
  if (sock.destroyed) return;
  try {
    sock.write(encodeNdjson({ kind: 'fatal', code, message }));
  } catch {
    // best effort
  }
  sock.end();
  // If end() hangs (unread buffer on peer), force.
  setTimeout(() => sock.destroy(), 100).unref?.();
}

function verifyToken(client: string, expected: string): boolean {
  const a = Buffer.from(client, 'utf8');
  const b = Buffer.from(expected, 'utf8');
  if (a.length !== b.length) return false;
  return crypto.timingSafeEqual(a, b);
}

function defaultSessionId(): string {
  return crypto.randomUUID();
}
