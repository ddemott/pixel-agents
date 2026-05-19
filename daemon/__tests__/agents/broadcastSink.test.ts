import { EventEmitter } from 'events';
import { describe, expect, it } from 'vitest';

import { BroadcastSink, SUBSCRIBER_QUEUE_MAX } from '../../src/agents/broadcastSink.js';
import { FrameDecoder } from '../../src/rpc/framing.js';

class MockSocket extends EventEmitter {
  readonly chunks: Buffer[] = [];
  destroyed = false;
  writable = true;
  /** When true, write() returns false to simulate kernel high-water-mark. */
  blocked = false;
  write(buf: Buffer): boolean {
    this.chunks.push(buf);
    return !this.blocked;
  }
  /** Decode every chunk we've received and concatenate the frames. */
  decode(): ReturnType<FrameDecoder['drain']> {
    const dec = new FrameDecoder();
    for (const c of this.chunks) dec.push(c);
    return dec.drain();
  }
  topics(): string[] {
    return this.decode().flatMap((f) => {
      if (f.kind !== 'ndjson') return [];
      const obj = JSON.parse(f.line) as { topic: string };
      return [obj.topic];
    });
  }
}

function asMock(s: MockSocket): import('net').Socket {
  return s as unknown as import('net').Socket;
}

describe('BroadcastSink', () => {
  it('fans out a single event to every registered connection', () => {
    const sink = new BroadcastSink();
    const a = new MockSocket();
    const b = new MockSocket();
    sink.register(asMock(a));
    sink.register(asMock(b));
    expect(sink.size()).toBe(2);

    sink.post({ type: 'agentToolStart', id: 7, toolName: 'Read' });

    for (const sock of [a, b]) {
      const frames = sock.decode();
      expect(frames).toHaveLength(1);
      expect(frames[0].kind).toBe('ndjson');
      if (frames[0].kind === 'ndjson') {
        const evt = JSON.parse(frames[0].line) as {
          kind: string;
          topic: string;
          seq: number;
          ts: number;
          data: { id: number; toolName: string };
        };
        expect(evt.kind).toBe('evt');
        expect(evt.topic).toBe('agentToolStart');
        expect(evt.seq).toBe(1);
        expect(typeof evt.ts).toBe('number');
        expect(evt.data.id).toBe(7);
        expect(evt.data.toolName).toBe('Read');
      }
    }
  });

  it('assigns monotonic seq per topic', () => {
    const sink = new BroadcastSink();
    const a = new MockSocket();
    sink.register(asMock(a));

    sink.post({ type: 'agentToolStart', id: 1 });
    sink.post({ type: 'agentToolStart', id: 2 });
    sink.post({ type: 'agentStatus', id: 1, status: 'idle' });
    sink.post({ type: 'agentToolStart', id: 3 });

    const seqs = a.decode().map((f) => {
      if (f.kind !== 'ndjson') return null;
      const evt = JSON.parse(f.line) as { topic: string; seq: number };
      return [evt.topic, evt.seq] as [string, number];
    });
    expect(seqs).toEqual([
      ['agentToolStart', 1],
      ['agentToolStart', 2],
      ['agentStatus', 1],
      ['agentToolStart', 3],
    ]);
  });

  it('skips destroyed / unwritable sockets without throwing', () => {
    const sink = new BroadcastSink();
    const live = new MockSocket();
    const dead = new MockSocket();
    sink.register(asMock(live));
    sink.register(asMock(dead));
    dead.destroyed = true;

    sink.post({ type: 'agentStatus', id: 1, status: 'idle' });

    expect(live.decode()).toHaveLength(1);
    expect(dead.decode()).toHaveLength(0);
  });

  it('unregisters automatically on socket close', () => {
    const sink = new BroadcastSink();
    const sock = new MockSocket();
    sink.register(asMock(sock));
    expect(sink.size()).toBe(1);
    sock.emit('close');
    expect(sink.size()).toBe(0);
  });

  describe('emitTo (per-agent scope)', () => {
    it('delivers only to clients subscribed to that agent', () => {
      const sink = new BroadcastSink();
      const focused = new MockSocket();
      const focusedSubs = new Set<string>(['agent:7']);
      const other = new MockSocket();
      const otherSubs = new Set<string>(['agent:99']);
      sink.register(asMock(focused), focusedSubs);
      sink.register(asMock(other), otherSubs);

      sink.emitTo(7, { type: 'agentStatus', id: 7, status: 'active' });

      expect(focused.topics()).toEqual(['agentStatus']);
      expect(other.topics()).toEqual([]);
    });

    it('honours agent:* wildcard', () => {
      const sink = new BroadcastSink();
      const all = new MockSocket();
      sink.register(asMock(all), new Set(['agent:*']));
      sink.emitTo(1, { type: 'agentStatus', id: 1, status: 'idle' });
      sink.emitTo(2, { type: 'agentStatus', id: 2, status: 'idle' });
      expect(all.topics()).toEqual(['agentStatus', 'agentStatus']);
    });

    it('reaches unfiltered clients (no agent: entries in their filter)', () => {
      const sink = new BroadcastSink();
      const unfiltered = new MockSocket();
      const topicOnly = new MockSocket();
      sink.register(asMock(unfiltered), new Set());
      sink.register(asMock(topicOnly), new Set(['agentStatus']));

      sink.emitTo(42, { type: 'agentStatus', id: 42, status: 'active' });

      expect(unfiltered.topics()).toEqual(['agentStatus']);
      // topicOnly subscribed by topic name — no agent: filter present means
      // they implicitly want every agent's events.
      expect(topicOnly.topics()).toEqual(['agentStatus']);
    });

    it('post bypasses the agent filter', () => {
      const sink = new BroadcastSink();
      const sock = new MockSocket();
      // Client subscribed only to agent:7, but a broadcast post should still
      // reach them since post is not agent-scoped.
      sink.register(asMock(sock), new Set(['agent:7']));
      sink.post({ type: 'layout.changed', source: 'file', layout: null });
      expect(sock.topics()).toEqual(['layout.changed']);
    });
  });

  describe('backpressure', () => {
    it('marks the connection paused and queues frames when write returns false', () => {
      const sink = new BroadcastSink();
      const sock = new MockSocket();
      let paused = 0;
      let resumed = 0;
      const id = sink.register(asMock(sock), new Set(), {
        onPause: () => paused++,
        onResume: () => resumed++,
      });

      sock.blocked = true;
      sink.post({ type: 'agentStatus', id: 1, status: 'active' });
      expect(sink.isPaused(id)).toBe(true);
      expect(paused).toBe(1);

      // Subsequent posts queue without hitting the socket.
      const writeCount = sock.chunks.length;
      sink.post({ type: 'agentStatus', id: 2, status: 'active' });
      expect(sock.chunks.length).toBe(writeCount);

      // Unblock + drain — queued frames flush, onResume fires.
      sock.blocked = false;
      sock.emit('drain');
      expect(sink.isPaused(id)).toBe(false);
      expect(resumed).toBe(1);
      expect(sock.topics()).toEqual(['agentStatus', 'agentStatus']);
    });

    it('drops oldest frames once the bounded queue overflows', () => {
      const sink = new BroadcastSink();
      const sock = new MockSocket();
      const id = sink.register(asMock(sock));
      sock.blocked = true;

      // First post pauses + writes one frame onto the kernel buffer.
      sink.post({ type: 'agentToolStart', id: 0 });
      // Now queue exactly SUBSCRIBER_QUEUE_MAX + extras to force eviction.
      for (let i = 1; i <= SUBSCRIBER_QUEUE_MAX + 5; i++) {
        sink.post({ type: 'agentToolStart', id: i });
      }
      expect(sink.droppedFrames(id)).toBe(5);

      sock.blocked = false;
      sock.emit('drain');
      // 1 initial write + SUBSCRIBER_QUEUE_MAX queued survivors.
      expect(sock.topics()).toHaveLength(1 + SUBSCRIBER_QUEUE_MAX);
    });

    it('discards the queue if the socket dies during flush', () => {
      const sink = new BroadcastSink();
      const sock = new MockSocket();
      sink.register(asMock(sock));
      sock.blocked = true;
      sink.post({ type: 'agentStatus', id: 1 });
      sink.post({ type: 'agentStatus', id: 2 });

      // Socket dies before drain — flush should bail without throwing.
      sock.destroyed = true;
      sock.writable = false;
      sock.emit('drain');
      // Only the first (pre-block) write made it onto the buffer; the rest
      // were queued and dropped on death.
      expect(sock.chunks).toHaveLength(1);
    });
  });
});
