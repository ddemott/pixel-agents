import * as fs from 'fs';
import * as net from 'net';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { attachConnection, type ConnectionContext } from '../../src/rpc/connection.js';
import { type DispatchContext, MethodRegistry } from '../../src/rpc/dispatch.js';
import { encodeNdjson, encodePtyOut, type Frame, FrameDecoder } from '../../src/rpc/framing.js';
import { type ClientCapabilities, type Hello, PROTO_VERSION } from '../../src/rpc/wire.js';

let tmpDir: string;
let socketPath: string;
let server: net.Server;

const VALID_TOKEN = 'a'.repeat(64);

const CAPS: ClientCapabilities = {
  rendering: '256',
  cols: 80,
  rows: 24,
  cellPx: { w: 8, h: 16 },
  bracketedPaste: true,
  mouse: true,
};

function emptyDispatchContext(): DispatchContext {
  // Minimal stub for tests that only check framing/auth — handlers are unreachable.
  return {
    ours: { processId: 1, bootId: 'test-boot' },
    sink: { post: () => {}, register: () => 0, unregister: () => {}, size: () => 0 } as never,
    agents: { forCwd: () => [] } as never,
    layoutDebouncer: { schedule: () => {}, flushNow: () => {}, dispose: () => {} } as never,
    state: { layout: null, config: { externalAssetDirectories: [] } },
    triggerShutdown: () => {},
  };
}

function makeContext(overrides: Partial<ConnectionContext> = {}): ConnectionContext {
  return {
    expectedToken: VALID_TOKEN,
    bootId: '00000000-0000-0000-0000-000000000000',
    daemonVersion: 'test',
    buildWorldSnapshot: () => ({
      schemaVersion: 1,
      worldSeed: 0,
      layout: null,
      assets: { catalog: [], characters: [], floors: [], walls: [] },
      agents: [],
    }),
    registry: new MethodRegistry(),
    dispatchContext: emptyDispatchContext(),
    ...overrides,
  };
}

function startServer(ctx: ConnectionContext): Promise<void> {
  return new Promise((resolve, reject) => {
    server = net.createServer((sock) => attachConnection(sock, ctx));
    server.once('error', reject);
    server.listen(socketPath, () => resolve());
  });
}

function stopServer(): Promise<void> {
  return new Promise((resolve) => {
    if (!server) return resolve();
    server.close(() => resolve());
  });
}

function connect(): Promise<net.Socket> {
  return new Promise((resolve, reject) => {
    const sock = net.createConnection(socketPath);
    sock.once('connect', () => resolve(sock));
    sock.once('error', reject);
  });
}

/** Read frames from a client socket until either `predicate` matches or socket closes. */
function collectFramesUntil(
  sock: net.Socket,
  predicate: (frames: Frame[]) => boolean,
  timeoutMs = 1000,
): Promise<{ frames: Frame[]; closed: boolean }> {
  return new Promise((resolve, reject) => {
    const decoder = new FrameDecoder();
    const collected: Frame[] = [];
    let closed = false;
    const finish = (err?: Error) => {
      sock.removeAllListeners('data');
      sock.removeAllListeners('close');
      sock.removeAllListeners('error');
      clearTimeout(timer);
      if (err) reject(err);
      else resolve({ frames: collected, closed });
    };
    const timer = setTimeout(() => finish(new Error('timeout')), timeoutMs);
    sock.on('data', (chunk: Buffer) => {
      try {
        decoder.push(chunk);
      } catch (e) {
        finish(e instanceof Error ? e : new Error(String(e)));
        return;
      }
      for (const f of decoder.drain()) collected.push(f);
      if (predicate(collected)) finish();
    });
    sock.on('close', () => {
      closed = true;
      finish();
    });
    sock.on('error', () => {
      closed = true;
      finish();
    });
  });
}

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-rpc-'));
  socketPath = path.join(tmpDir, 'sock');
});

afterEach(async () => {
  await stopServer();
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

describe('connection: handshake', () => {
  it('accepts hello with correct token and replies helloAck with inline world', async () => {
    await startServer(makeContext());
    const sock = await connect();
    const hello: Hello = {
      kind: 'hello',
      token: VALID_TOKEN,
      clientVersion: 'test-1',
      protoVersion: PROTO_VERSION,
      capabilities: CAPS,
    };
    sock.write(encodeNdjson(hello));
    const { frames } = await collectFramesUntil(sock, (fs) => fs.length >= 1);
    expect(frames).toHaveLength(1);
    expect(frames[0].kind).toBe('ndjson');
    if (frames[0].kind === 'ndjson') {
      const ack = JSON.parse(frames[0].line) as { kind: string; bootId: string; world: unknown };
      expect(ack.kind).toBe('helloAck');
      expect(ack.bootId).toBe('00000000-0000-0000-0000-000000000000');
      expect(ack.world).toBeDefined();
    }
    sock.destroy();
  });

  it('rejects hello with bad token and closes', async () => {
    await startServer(makeContext());
    const sock = await connect();
    const hello: Hello = {
      kind: 'hello',
      token: 'b'.repeat(64),
      clientVersion: 'test-1',
      protoVersion: PROTO_VERSION,
      capabilities: CAPS,
    };
    sock.write(encodeNdjson(hello));
    const { frames, closed } = await collectFramesUntil(sock, () => false, 800);
    expect(closed).toBe(true);
    // Daemon emits a fatal NDJSON before closing.
    const fatal = frames.find((f) => f.kind === 'ndjson');
    if (fatal && fatal.kind === 'ndjson') {
      const obj = JSON.parse(fatal.line) as { kind: string; code: string };
      expect(obj.kind).toBe('fatal');
      expect(obj.code).toBe('bad_token');
    }
  });

  it('rejects proto version mismatch', async () => {
    await startServer(makeContext());
    const sock = await connect();
    const hello: Hello = {
      kind: 'hello',
      token: VALID_TOKEN,
      clientVersion: 'test',
      protoVersion: 999,
      capabilities: CAPS,
    };
    sock.write(encodeNdjson(hello));
    const { closed } = await collectFramesUntil(sock, () => false, 800);
    expect(closed).toBe(true);
  });

  it('rejects non-hello first message and closes', async () => {
    await startServer(makeContext());
    const sock = await connect();
    sock.write(encodeNdjson({ kind: 'req', reqId: 1, method: 'agent.list', params: {} }));
    const { frames, closed } = await collectFramesUntil(sock, () => false, 800);
    expect(closed).toBe(true);
    const fatal = frames.find((f) => f.kind === 'ndjson');
    if (fatal && fatal.kind === 'ndjson') {
      const obj = JSON.parse(fatal.line) as { code: string };
      expect(obj.code).toBe('expected_hello');
    }
  });

  it('rejects binary frame before hello and closes', async () => {
    await startServer(makeContext());
    const sock = await connect();
    sock.write(encodePtyOut(1, Buffer.from('nope')));
    const { closed } = await collectFramesUntil(sock, () => false, 800);
    expect(closed).toBe(true);
  });

  it('replies unknown_method for an unregistered method after handshake', async () => {
    await startServer(makeContext());
    const sock = await connect();
    const hello: Hello = {
      kind: 'hello',
      token: VALID_TOKEN,
      clientVersion: 'test',
      protoVersion: PROTO_VERSION,
      capabilities: CAPS,
    };
    sock.write(encodeNdjson(hello));
    // wait for ack
    await collectFramesUntil(sock, (fs) => fs.length >= 1);
    sock.write(encodeNdjson({ kind: 'req', reqId: 7, method: 'agent.list', params: {} }));
    const { frames } = await collectFramesUntil(sock, (fs) =>
      fs.some((f) => {
        if (f.kind !== 'ndjson') return false;
        const obj = JSON.parse(f.line) as { kind: string; reqId?: number };
        return obj.kind === 'res' && obj.reqId === 7;
      }),
    );
    const resFrame = frames.find((f) => f.kind === 'ndjson' && JSON.parse(f.line).reqId === 7);
    expect(resFrame).toBeDefined();
    if (resFrame && resFrame.kind === 'ndjson') {
      const obj = JSON.parse(resFrame.line) as {
        ok: boolean;
        error: { code: string };
      };
      expect(obj.ok).toBe(false);
      expect(obj.error.code).toBe('unknown_method');
    }
    sock.destroy();
  });
});
