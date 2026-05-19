import { describe, expect, it } from 'vitest';

import {
  BINARY_MAX_FRAME,
  encodeAsset,
  encodeNdjson,
  encodePtyIn,
  encodePtyOut,
  FrameDecoder,
  FramingError,
  NDJSON_MAX_LINE,
} from '../../src/rpc/framing.js';

function drainAll(dec: FrameDecoder, chunks: Buffer[]): ReturnType<FrameDecoder['drain']> {
  for (const c of chunks) dec.push(c);
  return dec.drain();
}

describe('framing: NDJSON', () => {
  it('encodes + decodes a single JSON object', () => {
    const enc = encodeNdjson({ hello: 'world', n: 42 });
    const dec = new FrameDecoder();
    dec.push(enc);
    const frames = dec.drain();
    expect(frames).toHaveLength(1);
    expect(frames[0]).toEqual({ kind: 'ndjson', line: '{"hello":"world","n":42}' });
  });

  it('decodes multiple NDJSON frames concatenated', () => {
    const buf = Buffer.concat([
      encodeNdjson({ a: 1 }),
      encodeNdjson({ b: 2 }),
      encodeNdjson({ c: 3 }),
    ]);
    const dec = new FrameDecoder();
    dec.push(buf);
    const frames = dec.drain();
    expect(frames.map((f) => (f.kind === 'ndjson' ? f.line : null))).toEqual([
      '{"a":1}',
      '{"b":2}',
      '{"c":3}',
    ]);
  });

  it('handles split chunks (random fragmentation, NDJSON)', () => {
    const buf = Buffer.concat([
      encodeNdjson({ a: 1 }),
      encodeNdjson({ b: 'two' }),
      encodeNdjson({ c: [1, 2, 3] }),
    ]);
    // Fragment into byte-size chunks (worst case).
    const chunks: Buffer[] = [];
    for (let i = 0; i < buf.length; i++) chunks.push(buf.slice(i, i + 1));
    const dec = new FrameDecoder();
    const frames = drainAll(dec, chunks);
    expect(frames).toHaveLength(3);
  });

  it('rejects an NDJSON line that exceeds 256 KB cap before newline', () => {
    const dec = new FrameDecoder();
    // 256 KB + 1 payload bytes with no newline yet
    const oversize = Buffer.alloc(NDJSON_MAX_LINE + 2);
    oversize[0] = 0x00; // tag
    oversize.fill(0x20, 1); // spaces, no newline
    expect(() => dec.push(oversize)).toThrow(FramingError);
  });

  it('encodeNdjson rejects oversize payload', () => {
    const big = { s: 'x'.repeat(NDJSON_MAX_LINE) };
    expect(() => encodeNdjson(big)).toThrow(FramingError);
  });
});

describe('framing: PTY frames', () => {
  it('encodes + decodes a PTY outbound frame', () => {
    const payload = Buffer.from('hello pty');
    const enc = encodePtyOut(7, payload);
    const dec = new FrameDecoder();
    dec.push(enc);
    const frames = dec.drain();
    expect(frames).toHaveLength(1);
    expect(frames[0].kind).toBe('ptyOut');
    if (frames[0].kind === 'ptyOut') {
      expect(frames[0].streamId).toBe(7);
      expect(frames[0].bytes.equals(payload)).toBe(true);
    }
  });

  it('encodes + decodes a PTY inbound frame', () => {
    const payload = Buffer.from([0x01, 0x02, 0x03, 0x04]);
    const dec = new FrameDecoder();
    dec.push(encodePtyIn(0xdeadbeef >>> 0, payload));
    const frames = dec.drain();
    expect(frames[0].kind).toBe('ptyIn');
    if (frames[0].kind === 'ptyIn') {
      expect(frames[0].streamId).toBe(0xdeadbeef);
      expect(frames[0].bytes.equals(payload)).toBe(true);
    }
  });

  it('handles split chunks (PTY frame across multiple pushes)', () => {
    const payload = Buffer.alloc(1024);
    for (let i = 0; i < payload.length; i++) payload[i] = i & 0xff;
    const enc = encodePtyOut(1, payload);
    const dec = new FrameDecoder();
    // Split at every prime-ish offset
    const cuts = [3, 7, 13, 100, 500, 900, enc.length];
    let prev = 0;
    for (const cut of cuts) {
      dec.push(enc.slice(prev, cut));
      prev = cut;
    }
    const frames = dec.drain();
    expect(frames).toHaveLength(1);
    if (frames[0].kind === 'ptyOut') {
      expect(frames[0].bytes.equals(payload)).toBe(true);
    }
  });

  it('rejects PTY frame exceeding 1 MB cap', () => {
    const dec = new FrameDecoder();
    // Manually craft a header claiming len > cap
    const hdr = Buffer.alloc(9);
    hdr[0] = 0x01;
    hdr.writeUInt32BE(0, 1);
    hdr.writeUInt32BE(BINARY_MAX_FRAME + 1, 5);
    expect(() => dec.push(hdr)).toThrow(FramingError);
  });

  it('encodePtyOut rejects oversize payload', () => {
    expect(() => encodePtyOut(0, Buffer.alloc(BINARY_MAX_FRAME + 1))).toThrow(FramingError);
  });
});

describe('framing: asset blobs', () => {
  it('encodes + decodes a single asset frame with final=true', () => {
    const payload = Buffer.from([10, 20, 30]);
    const dec = new FrameDecoder();
    dec.push(encodeAsset(42, 2, payload, true));
    const frames = dec.drain();
    expect(frames).toHaveLength(1);
    expect(frames[0]).toEqual({
      kind: 'asset',
      assetId: 42,
      tier: 2,
      final: true,
      bytes: payload,
    });
  });

  it('multi-frame asset: high-bit-of-tier marks EOF', () => {
    const dec = new FrameDecoder();
    const p1 = Buffer.from('chunk1');
    const p2 = Buffer.from('chunk2');
    const p3 = Buffer.from('chunk3-final');
    dec.push(encodeAsset(7, 1, p1, false));
    dec.push(encodeAsset(7, 1, p2, false));
    dec.push(encodeAsset(7, 1, p3, true));
    const frames = dec.drain();
    expect(frames).toHaveLength(3);
    expect(frames.every((f) => f.kind === 'asset')).toBe(true);
    if (frames[0].kind === 'asset') {
      expect(frames[0].final).toBe(false);
      expect(frames[0].tier).toBe(1);
      expect(frames[0].assetId).toBe(7);
    }
    if (frames[2].kind === 'asset') {
      expect(frames[2].final).toBe(true);
      expect(frames[2].bytes.toString()).toBe('chunk3-final');
    }
  });

  it('rejects asset tier out of range', () => {
    expect(() => encodeAsset(1, 128, Buffer.alloc(0), true)).toThrow(FramingError);
    expect(() => encodeAsset(1, -1, Buffer.alloc(0), true)).toThrow(FramingError);
  });
});

describe('framing: unknown tag', () => {
  it('throws on unknown tag byte', () => {
    const dec = new FrameDecoder();
    expect(() => dec.push(Buffer.from([0x77, 0x00]))).toThrow(/unknown frame tag/);
  });
});

describe('framing: fuzz', () => {
  it('survives random chunked input across all 4 channels', () => {
    // Build a buffer with a mix of frame kinds.
    const parts: Buffer[] = [];
    const expected: Array<{ kind: string }> = [];
    for (let i = 0; i < 50; i++) {
      const r = i % 4;
      if (r === 0) {
        parts.push(encodeNdjson({ i }));
        expected.push({ kind: 'ndjson' });
      } else if (r === 1) {
        parts.push(encodePtyOut(i, Buffer.from(`out-${i}`)));
        expected.push({ kind: 'ptyOut' });
      } else if (r === 2) {
        parts.push(encodeAsset(i, i & 0x7f, Buffer.from(`asset-${i}`), (i & 1) === 0));
        expected.push({ kind: 'asset' });
      } else {
        parts.push(encodePtyIn(i, Buffer.from(`in-${i}`)));
        expected.push({ kind: 'ptyIn' });
      }
    }
    const buf = Buffer.concat(parts);
    // Fragment randomly (deterministic seed via index).
    const dec = new FrameDecoder();
    let off = 0;
    let step = 1;
    while (off < buf.length) {
      const end = Math.min(buf.length, off + step);
      dec.push(buf.slice(off, end));
      off = end;
      step = (step * 7 + 3) % 17 || 1;
    }
    const frames = dec.drain();
    expect(frames.map((f) => f.kind)).toEqual(expected.map((e) => e.kind));
  });
});
