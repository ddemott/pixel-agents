import { EventEmitter } from 'events';
import { describe, expect, it } from 'vitest';

import { BroadcastSink } from '../../src/agents/broadcastSink.js';
import { FrameDecoder } from '../../src/rpc/framing.js';

class MockSocket extends EventEmitter {
  readonly chunks: Buffer[] = [];
  destroyed = false;
  writable = true;
  write(buf: Buffer): boolean {
    this.chunks.push(buf);
    return true;
  }
  /** Decode every chunk we've received and concatenate the frames. */
  decode(): ReturnType<FrameDecoder['drain']> {
    const dec = new FrameDecoder();
    for (const c of this.chunks) dec.push(c);
    return dec.drain();
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
});
