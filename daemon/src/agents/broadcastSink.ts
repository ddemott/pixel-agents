import type { Socket } from 'net';

import type { AgentEvent, AgentEventSink } from '../../../src/messageSender.js';
import { encodeNdjson, FramingError } from '../rpc/framing.js';
import type { Evt } from '../rpc/wire.js';

/**
 * Maximum number of encoded frames a paused connection may queue before older
 * frames are dropped. Keeps daemon memory bounded if a client wedges its UDS
 * read side without closing. PTY pumps (Day 13-14) gate themselves on
 * `isPaused()` instead of queueing here.
 */
export const SUBSCRIBER_QUEUE_MAX = 256;

export interface BackpressureCallbacks {
  /** Fires the first time the kernel buffer signals high-water-mark. */
  onPause?: () => void;
  /** Fires after the queue has fully drained back to a writable kernel buffer. */
  onResume?: () => void;
}

interface Subscriber {
  connId: number;
  sock: Socket;
  /**
   * Topic filter shared with the connection scope by reference. An empty set
   * means "no filter" (deliver all topics); a non-empty set restricts to its
   * entries. `"*"` is also honoured as the explicit all-topics wildcard.
   */
  subscriptions: Set<string>;
  paused: boolean;
  queue: Buffer[];
  droppedFrames: number;
  callbacks: BackpressureCallbacks;
}

/**
 * AgentEventSink implementation that fans events out to every authed RPC client.
 *
 * Each call to `post(event)` is wrapped into an `Evt` envelope (kind:'evt') with
 * a topic equal to the event type, a per-topic monotonic `seq`, and a daemon
 * timestamp. The full original event object is shipped as `data`, so clients
 * can reuse the payloads the extension's webview already consumes.
 *
 * Day 9-10 additions:
 *  - `emitTo(agentId, event)` only reaches connections that subscribed to
 *    `agent:<id>`, `agent:*`, `*`, or to no `agent:` filter at all.
 *  - Per-connection backpressure: when `sock.write()` returns false, the
 *    subscriber is marked paused and subsequent frames are queued (bounded
 *    ring; oldest dropped on overflow). When the socket emits `'drain'`, the
 *    queue is flushed and `onResume` fires. PTY hosts (Day 13-14) read
 *    `isPaused()` to pause their `pty.read()` loops cooperatively.
 */
export class BroadcastSink implements AgentEventSink {
  private readonly conns = new Map<number, Subscriber>();
  private nextConnId = 1;
  private readonly seqByTopic = new Map<string, number>();

  /**
   * Register a freshly-authed socket. The subscription set is shared by
   * reference so future `subscribe` RPCs mutate it in place. Optional
   * backpressure callbacks fire when the connection pauses/resumes so PTY
   * pumps (or other slow producers) can stop reading.
   */
  register(
    sock: Socket,
    subscriptions: Set<string> = new Set(),
    callbacks: BackpressureCallbacks = {},
  ): number {
    const connId = this.nextConnId++;
    const sub: Subscriber = {
      connId,
      sock,
      subscriptions,
      paused: false,
      queue: [],
      droppedFrames: 0,
      callbacks,
    };
    this.conns.set(connId, sub);
    const cleanup = (): void => {
      this.conns.delete(connId);
    };
    sock.once('close', cleanup);
    sock.once('error', cleanup);
    return connId;
  }

  /** Manual unregister. Idempotent. */
  unregister(connId: number): void {
    this.conns.delete(connId);
  }

  /** Number of live broadcast targets. */
  size(): number {
    return this.conns.size;
  }

  /** True when the connection's kernel buffer is saturated. PTY pumps poll this. */
  isPaused(connId: number): boolean {
    return this.conns.get(connId)?.paused ?? false;
  }

  /** Diagnostic — how many frames have been dropped due to overflow. */
  droppedFrames(connId: number): number {
    return this.conns.get(connId)?.droppedFrames ?? 0;
  }

  post(event: AgentEvent): void {
    const frame = this.encode(event);
    if (!frame) return;
    for (const sub of this.conns.values()) {
      if (!matchesTopic(sub.subscriptions, event.type)) continue;
      this.writeTo(sub, frame);
    }
  }

  emitTo(agentId: number, event: AgentEvent): void {
    const frame = this.encode(event);
    if (!frame) return;
    for (const sub of this.conns.values()) {
      if (!matchesTopic(sub.subscriptions, event.type)) continue;
      if (!matchesAgent(sub.subscriptions, agentId)) continue;
      this.writeTo(sub, frame);
    }
  }

  /** Encode + tag with topic + bump per-topic seq. Returns null on oversize. */
  private encode(event: AgentEvent): Buffer | null {
    const topic = event.type;
    const seq = (this.seqByTopic.get(topic) ?? 0) + 1;
    this.seqByTopic.set(topic, seq);
    const evt: Evt = {
      kind: 'evt',
      topic,
      seq,
      ts: Date.now(),
      data: event,
    };
    try {
      return encodeNdjson(evt);
    } catch (e) {
      if (e instanceof FramingError) {
        console.error(`[BroadcastSink] dropped oversize event topic=${topic}: ${e.message}`);
        return null;
      }
      throw e;
    }
  }

  private writeTo(sub: Subscriber, frame: Buffer): void {
    if (sub.sock.destroyed || !sub.sock.writable) return;
    if (sub.paused) {
      this.enqueue(sub, frame);
      return;
    }
    const ok = sub.sock.write(frame);
    if (!ok) {
      sub.paused = true;
      sub.sock.once('drain', () => this.flushQueue(sub));
      sub.callbacks.onPause?.();
    }
  }

  private enqueue(sub: Subscriber, frame: Buffer): void {
    if (sub.queue.length >= SUBSCRIBER_QUEUE_MAX) {
      sub.queue.shift();
      sub.droppedFrames++;
    }
    sub.queue.push(frame);
  }

  private flushQueue(sub: Subscriber): void {
    while (sub.queue.length > 0) {
      if (sub.sock.destroyed || !sub.sock.writable) {
        sub.queue.length = 0;
        return;
      }
      const next = sub.queue.shift()!;
      const ok = sub.sock.write(next);
      if (!ok) {
        sub.sock.once('drain', () => this.flushQueue(sub));
        return;
      }
    }
    sub.paused = false;
    sub.callbacks.onResume?.();
  }
}

function matchesTopic(subs: Set<string>, topic: string): boolean {
  if (subs.size === 0) return true; // unfiltered default
  if (subs.has('*')) return true;
  if (subs.has(topic)) return true;
  // Topic filters and agent filters share the same set; if the only entries
  // are `agent:*` selectors, the client still wants topic events through.
  for (const s of subs) {
    if (!s.startsWith('agent:')) return false;
  }
  return true;
}

function matchesAgent(subs: Set<string>, agentId: number): boolean {
  if (subs.size === 0) return true; // unfiltered default
  if (subs.has('*')) return true;
  if (subs.has('agent:*')) return true;
  if (subs.has(`agent:${agentId}`)) return true;
  // If the client has only topic-style filters (no `agent:` entries), they're
  // implicitly subscribed to every agent.
  for (const s of subs) {
    if (s.startsWith('agent:')) return false;
  }
  return true;
}
