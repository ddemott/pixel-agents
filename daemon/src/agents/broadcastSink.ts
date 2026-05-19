import type { Socket } from 'net';

import type { AgentEvent, AgentEventSink } from '../../../src/messageSender.js';
import { encodeNdjson, FramingError } from '../rpc/framing.js';
import type { Evt } from '../rpc/wire.js';

/**
 * AgentEventSink implementation that fans out events to every authed RPC client.
 *
 * Each call to `post(event)` is wrapped into an `Evt` envelope (kind:'evt') with
 * a topic equal to the event type, a per-topic monotonic `seq`, and a daemon
 * timestamp. The full original event object is shipped as `data`, so clients
 * can reuse the same payloads the extension's webview consumes.
 *
 * Day 5 scope: fan-out only. Per-agent scope (`emitTo(agentId, ...)`) and
 * backpressure on slow consumers land in Day 9-10.
 */
export class BroadcastSink implements AgentEventSink {
  private readonly conns = new Map<number, Socket>();
  private nextConnId = 1;
  private readonly seqByTopic = new Map<string, number>();

  /** Register a freshly-authed socket. Returns its connection id (for unregister). */
  register(sock: Socket): number {
    const id = this.nextConnId++;
    this.conns.set(id, sock);
    const cleanup = (): void => {
      this.conns.delete(id);
    };
    sock.once('close', cleanup);
    sock.once('error', cleanup);
    return id;
  }

  /** Manual unregister. Idempotent. */
  unregister(connId: number): void {
    this.conns.delete(connId);
  }

  /** Number of live broadcast targets. */
  size(): number {
    return this.conns.size;
  }

  post(event: AgentEvent): void {
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

    let frame: Buffer;
    try {
      frame = encodeNdjson(evt);
    } catch (e) {
      if (e instanceof FramingError) {
        console.error(`[BroadcastSink] dropped oversize event topic=${topic}: ${e.message}`);
        return;
      }
      throw e;
    }

    for (const sock of this.conns.values()) {
      if (sock.destroyed || !sock.writable) continue;
      sock.write(frame);
    }
  }
}
