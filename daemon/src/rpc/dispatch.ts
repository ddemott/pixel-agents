import type { Socket } from 'net';

import type { BroadcastSink } from '../agents/broadcastSink.js';
import type { AgentsRegistry } from '../agents/registry.js';
import type { PixelAgentsConfig } from '../config/persistence.js';
import type { Layout, LayoutSaveDebouncer } from '../layout/persistence.js';
import type { WriterTag } from '../persistence/writerTag.js';
import type { Res, WireError } from './wire.js';

/**
 * Per-connection mutable state exposed to method handlers. Anything that lives
 * for the duration of a single client connection (subscriptions, focus
 * targets) belongs here; daemon-wide state (sink, registry, debouncer) lands
 * on `DispatchContext`.
 */
export interface ConnectionScope {
  /** Stable id assigned at handshake. */
  sessionId: string;
  /** Topics the client wants to receive on its UDS stream. Default: all. */
  subscriptions: Set<string>;
  /** Raw socket — handlers that need to write directly (rare) reach for it. */
  sock: Socket;
}

/**
 * Daemon-wide handles. Constructed once at boot and passed to every dispatch.
 * `triggerShutdown` is a callback the daemon registers so `daemon.shutdown`
 * can ask main to begin the SIGTERM-equivalent cleanup.
 */
export interface DispatchContext {
  ours: WriterTag;
  sink: BroadcastSink;
  agents: AgentsRegistry;
  layoutDebouncer: LayoutSaveDebouncer;
  /** Mutable refs the server.ts boot wires up so handlers always see fresh state. */
  state: {
    layout: Layout | null;
    config: PixelAgentsConfig;
  };
  /** Set when layout or config has been mutated by a handler — broadcasters use it. */
  triggerShutdown: () => void;
}

export type Handler = (
  params: unknown,
  scope: ConnectionScope,
  ctx: DispatchContext,
) => Res | Promise<Res>;

/**
 * Registry of method name → handler. Built at boot and frozen; new entries
 * are not added at runtime.
 */
export class MethodRegistry {
  private readonly handlers = new Map<string, Handler>();

  register(method: string, handler: Handler): void {
    if (this.handlers.has(method)) {
      throw new Error(`Duplicate RPC handler: ${method}`);
    }
    this.handlers.set(method, handler);
  }

  has(method: string): boolean {
    return this.handlers.has(method);
  }

  async dispatch(
    reqId: number,
    method: string,
    params: unknown,
    scope: ConnectionScope,
    ctx: DispatchContext,
  ): Promise<Res> {
    const handler = this.handlers.get(method);
    if (!handler) {
      return makeError(reqId, 'unknown_method', `no handler registered for '${method}'`);
    }
    try {
      const result = await handler(params, scope, ctx);
      // Handlers return `Res` with `reqId: 0`; the dispatcher backfills the
      // real reqId so handlers don't have to know it.
      return { ...result, reqId } as Res;
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      return makeError(reqId, 'handler_threw', message);
    }
  }
}

export function ok(data: unknown): Res {
  return { kind: 'res', reqId: 0, ok: true, data };
}

export function err(code: string, message: string): Res {
  return { kind: 'res', reqId: 0, ok: false, error: { code, message } };
}

function makeError(reqId: number, code: string, message: string): Res {
  const error: WireError = { code, message };
  return { kind: 'res', reqId, ok: false, error };
}
