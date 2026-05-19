/**
 * Channel-multiplexed framing for the daemon UDS / named-pipe transport.
 * See docs/tui-architecture.md §10 for the on-wire format.
 *
 *   0x00 NDJSON control     : <line>\n            (max 256 KB)
 *   0x01 PTY outbound        : streamId:u32_be len:u32_be bytes[len]  (len ≤ 1 MB)
 *   0x02 asset blob          : assetId:u32_be tier:u8 len:u32_be bytes[len] (len ≤ 1 MB)
 *                              high bit (0x80) of `tier` set on the final frame.
 *   0x03 PTY inbound (large) : streamId:u32_be len:u32_be bytes[len]  (len ≤ 1 MB)
 *
 * Decoder is streaming: feed it Buffer chunks; it returns zero-or-more parsed
 * frames per call. Encoder is one-shot per frame.
 */

export const TAG_NDJSON = 0x00;
export const TAG_PTY_OUT = 0x01;
export const TAG_ASSET = 0x02;
export const TAG_PTY_IN = 0x03;

export const NDJSON_MAX_LINE = 256 * 1024; // 256 KB
export const BINARY_MAX_FRAME = 1024 * 1024; // 1 MB

export type Frame =
  | { kind: 'ndjson'; line: string }
  | { kind: 'ptyOut'; streamId: number; bytes: Buffer }
  | { kind: 'ptyIn'; streamId: number; bytes: Buffer }
  | { kind: 'asset'; assetId: number; tier: number; final: boolean; bytes: Buffer };

export class FramingError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'FramingError';
  }
}

/**
 * Streaming decoder. Append bytes with `push(chunk)`, drain with `drain()`.
 * Stateful — keep one instance per socket.
 */
export class FrameDecoder {
  private buf: Buffer = Buffer.alloc(0);
  private pending: Frame[] = [];

  push(chunk: Buffer): void {
    this.buf = this.buf.length === 0 ? chunk : Buffer.concat([this.buf, chunk]);
    this.parse();
  }

  drain(): Frame[] {
    const out = this.pending;
    this.pending = [];
    return out;
  }

  /** Throws FramingError on protocol violation; caller should close the socket. */
  private parse(): void {
    while (this.buf.length > 0) {
      const tag = this.buf[0];

      if (tag === TAG_NDJSON) {
        // Search for newline starting at offset 1.
        const nl = this.buf.indexOf(0x0a, 1);
        if (nl === -1) {
          // Guard against an unterminated line that exceeds the cap.
          if (this.buf.length - 1 > NDJSON_MAX_LINE) {
            throw new FramingError(`NDJSON line exceeded ${NDJSON_MAX_LINE} bytes without newline`);
          }
          return; // wait for more bytes
        }
        const lineLen = nl - 1;
        if (lineLen > NDJSON_MAX_LINE) {
          throw new FramingError(`NDJSON line ${lineLen} bytes > ${NDJSON_MAX_LINE} cap`);
        }
        const line = this.buf.slice(1, nl).toString('utf8');
        this.buf = this.buf.slice(nl + 1);
        this.pending.push({ kind: 'ndjson', line });
        continue;
      }

      if (tag === TAG_PTY_OUT || tag === TAG_PTY_IN) {
        // 1 (tag) + 4 (streamId) + 4 (len) = 9 byte header
        if (this.buf.length < 9) return;
        const streamId = this.buf.readUInt32BE(1);
        const len = this.buf.readUInt32BE(5);
        if (len > BINARY_MAX_FRAME) {
          throw new FramingError(
            `PTY frame len ${len} > ${BINARY_MAX_FRAME} cap (tag=0x0${tag.toString(16)})`,
          );
        }
        if (this.buf.length < 9 + len) return;
        const bytes = this.buf.slice(9, 9 + len);
        this.buf = this.buf.slice(9 + len);
        this.pending.push(
          tag === TAG_PTY_OUT
            ? { kind: 'ptyOut', streamId, bytes }
            : { kind: 'ptyIn', streamId, bytes },
        );
        continue;
      }

      if (tag === TAG_ASSET) {
        // 1 (tag) + 4 (assetId) + 1 (tier) + 4 (len) = 10 byte header
        if (this.buf.length < 10) return;
        const assetId = this.buf.readUInt32BE(1);
        const tierByte = this.buf[5];
        const len = this.buf.readUInt32BE(6);
        if (len > BINARY_MAX_FRAME) {
          throw new FramingError(`Asset frame len ${len} > ${BINARY_MAX_FRAME} cap`);
        }
        if (this.buf.length < 10 + len) return;
        const final = (tierByte & 0x80) !== 0;
        const tier = tierByte & 0x7f;
        const bytes = this.buf.slice(10, 10 + len);
        this.buf = this.buf.slice(10 + len);
        this.pending.push({ kind: 'asset', assetId, tier, final, bytes });
        continue;
      }

      throw new FramingError(`unknown frame tag 0x${tag.toString(16).padStart(2, '0')}`);
    }
  }
}

/** Encode an NDJSON line. `obj` is JSON-stringified; rejects if oversize. */
export function encodeNdjson(obj: unknown): Buffer {
  const json = JSON.stringify(obj);
  const payload = Buffer.from(json, 'utf8');
  if (payload.length > NDJSON_MAX_LINE) {
    throw new FramingError(`NDJSON payload ${payload.length} > ${NDJSON_MAX_LINE} cap`);
  }
  const out = Buffer.alloc(1 + payload.length + 1);
  out[0] = TAG_NDJSON;
  payload.copy(out, 1);
  out[1 + payload.length] = 0x0a;
  return out;
}

function encodePty(tag: number, streamId: number, bytes: Buffer): Buffer {
  if (bytes.length > BINARY_MAX_FRAME) {
    throw new FramingError(`PTY payload ${bytes.length} > ${BINARY_MAX_FRAME} cap`);
  }
  const out = Buffer.alloc(9 + bytes.length);
  out[0] = tag;
  out.writeUInt32BE(streamId >>> 0, 1);
  out.writeUInt32BE(bytes.length, 5);
  bytes.copy(out, 9);
  return out;
}

export function encodePtyOut(streamId: number, bytes: Buffer): Buffer {
  return encodePty(TAG_PTY_OUT, streamId, bytes);
}

export function encodePtyIn(streamId: number, bytes: Buffer): Buffer {
  return encodePty(TAG_PTY_IN, streamId, bytes);
}

/** Single asset frame. For blobs > 1 MB, call repeatedly with `final=false` then once with `final=true`. */
export function encodeAsset(assetId: number, tier: number, bytes: Buffer, final: boolean): Buffer {
  if (tier < 0 || tier > 0x7f) {
    throw new FramingError(`asset tier ${tier} out of range (0..127)`);
  }
  if (bytes.length > BINARY_MAX_FRAME) {
    throw new FramingError(`asset payload ${bytes.length} > ${BINARY_MAX_FRAME} cap`);
  }
  const out = Buffer.alloc(10 + bytes.length);
  out[0] = TAG_ASSET;
  out.writeUInt32BE(assetId >>> 0, 1);
  out[5] = (final ? 0x80 : 0x00) | (tier & 0x7f);
  out.writeUInt32BE(bytes.length, 6);
  bytes.copy(out, 10);
  return out;
}
