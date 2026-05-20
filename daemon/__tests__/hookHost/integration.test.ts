import * as fs from 'fs';
import * as http from 'http';
import * as net from 'net';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { BroadcastSink } from '../../src/agents/broadcastSink.js';
import { DaemonHookBridge } from '../../src/hookHost/bridge.js';
import { type HookHTTPServerHandle, startHookServer } from '../../src/hookHost/server.js';
import { createNullLogger } from '../../src/logging/logger.js';
import { attachConnection, type ConnectionContext } from '../../src/rpc/connection.js';
import type { DispatchContext } from '../../src/rpc/dispatch.js';
import { encodeNdjson, type Frame, FrameDecoder } from '../../src/rpc/framing.js';
import { buildMethodRegistry } from '../../src/rpc/methods/index.js';
import { type Hello, PROTO_VERSION, type Req } from '../../src/rpc/wire.js';

/**
 * Phase 1 Day 12 integration test.
 *
 * Wires up the real BroadcastSink, DaemonHookBridge, and HTTP hook server
 * behind a real UDS listener that runs `attachConnection`. POSTs synthetic
 * hook payloads to the HTTP endpoint and asserts the broadcast events fan
 * out to the subscribed mock client. This is the same path a `claude` child
 * process would take through `claude-hook.js`, minus the script itself —
 * which is just a JSON POST shim.
 */

const TOKEN = 'b'.repeat(64);

let tmpDir: string;
let socketPath: string;
let udsServer: net.Server;
let hookHandle: HookHTTPServerHandle;
let sink: BroadcastSink;
let bridge: DaemonHookBridge;

beforeEach(async () => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-hook-int-'));
  socketPath = path.join(tmpDir, 'daemon.sock');

  const logger = createNullLogger();
  sink = new BroadcastSink();
  sink.setLogger(logger);
  bridge = new DaemonHookBridge(sink, logger);

  hookHandle = await startHookServer({
    token: TOKEN,
    onEvent: (providerId, event) => bridge.handleEvent(providerId, event),
    logger,
  });

  const registry = buildMethodRegistry();
  const ctx = makeDispatchContext();
  const connCtx: ConnectionContext = {
    expectedToken: TOKEN,
    bootId: 'test-boot',
    daemonVersion: 'test',
    buildWorldSnapshot: () => ({
      schemaVersion: 1,
      worldSeed: 0,
      layout: null,
      assets: { catalog: [], characterCount: 0, floorCount: 0, wallCount: 0 },
      agents: [],
    }),
    registry,
    dispatchContext: ctx,
    onAuthenticated: (authed, scope) => {
      sink.register(authed, scope.subscriptions);
    },
  };

  udsServer = net.createServer((s) => attachConnection(s, connCtx));
  await new Promise<void>((resolve, reject) => {
    udsServer.once('error', reject);
    udsServer.listen(socketPath, () => resolve());
  });
});

afterEach(async () => {
  await hookHandle.close();
  await new Promise<void>((resolve) => udsServer.close(() => resolve()));
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

function makeDispatchContext(): DispatchContext {
  return {
    ours: { processId: process.pid, bootId: 'test-boot' },
    sink,
    agents: { forCwd: () => [] } as never,
    layoutDebouncer: { schedule: () => {}, flushNow: () => {}, dispose: () => {} } as never,
    liveAgents: { get: () => undefined, list: () => [] } as never,
    hookBridge: bridge,
    logger: createNullLogger(),
    assetRegistry: { getCatalog: () => ({ assets: [] }), getPng: () => null } as never,
    state: { layout: null, config: { externalAssetDirectories: [], logLevel: 'info' } },
    triggerShutdown: () => {},
  };
}

interface ClientHandle {
  sock: net.Socket;
  evts: Array<{ topic: string; data: Record<string, unknown> }>;
  /** Wait until at least one event matches `predicate`, or reject after `timeoutMs`. */
  waitFor(
    predicate: (e: { topic: string; data: Record<string, unknown> }) => boolean,
    timeoutMs?: number,
  ): Promise<{ topic: string; data: Record<string, unknown> }>;
  close(): void;
}

async function connectClient(): Promise<ClientHandle> {
  const sock = await new Promise<net.Socket>((resolve, reject) => {
    const s = net.createConnection(socketPath);
    s.once('connect', () => resolve(s));
    s.once('error', reject);
  });

  const decoder = new FrameDecoder();
  const evts: ClientHandle['evts'] = [];
  const waiters: Array<{
    predicate: (e: { topic: string; data: Record<string, unknown> }) => boolean;
    resolve: (e: { topic: string; data: Record<string, unknown> }) => void;
    reject: (err: Error) => void;
    timer: ReturnType<typeof setTimeout>;
  }> = [];

  const onEvt = (e: { topic: string; data: Record<string, unknown> }): void => {
    evts.push(e);
    for (let i = waiters.length - 1; i >= 0; i--) {
      const w = waiters[i];
      if (w.predicate(e)) {
        clearTimeout(w.timer);
        waiters.splice(i, 1);
        w.resolve(e);
      }
    }
  };

  sock.on('data', (chunk: Buffer) => {
    decoder.push(chunk);
    for (const frame of decoder.drain()) handleFrame(frame, onEvt);
  });

  const hello: Hello = {
    kind: 'hello',
    token: TOKEN,
    clientVersion: 'integration-test',
    protoVersion: PROTO_VERSION,
    capabilities: {
      rendering: '256',
      cols: 80,
      rows: 24,
      cellPx: { w: 8, h: 16 },
      bracketedPaste: true,
      mouse: true,
    },
  };
  sock.write(encodeNdjson(hello));

  // Wait for helloAck.
  await new Promise<void>((resolve, reject) => {
    const t = setTimeout(() => reject(new Error('helloAck timeout')), 1000);
    const handler = (chunk: Buffer): void => {
      decoder.push(chunk);
      for (const frame of decoder.drain()) {
        if (frame.kind === 'ndjson') {
          const msg = JSON.parse(frame.line) as { kind?: string };
          if (msg.kind === 'helloAck') {
            clearTimeout(t);
            sock.removeListener('data', handler);
            resolve();
            return;
          }
        }
        handleFrame(frame, onEvt);
      }
    };
    sock.on('data', handler);
  });

  return {
    sock,
    evts,
    waitFor: (predicate, timeoutMs = 1500) =>
      new Promise((resolve, reject) => {
        const match = evts.find(predicate);
        if (match) return resolve(match);
        const timer = setTimeout(() => {
          const idx = waiters.findIndex((w) => w.timer === timer);
          if (idx >= 0) waiters.splice(idx, 1);
          reject(new Error('waitFor: predicate never matched'));
        }, timeoutMs);
        waiters.push({ predicate, resolve, reject, timer });
      }),
    close: () => sock.destroy(),
  };
}

function handleFrame(
  frame: Frame,
  onEvt: (e: { topic: string; data: Record<string, unknown> }) => void,
): void {
  if (frame.kind !== 'ndjson') return;
  const msg = JSON.parse(frame.line) as { kind?: string };
  if (msg.kind === 'evt') {
    const e = msg as unknown as { topic: string; data: Record<string, unknown> };
    onEvt({ topic: e.topic, data: e.data });
  }
}

async function postHook(provider: string, body: Record<string, unknown>): Promise<number> {
  return await new Promise<number>((resolve, reject) => {
    const data = Buffer.from(JSON.stringify(body));
    const req = http.request(
      {
        hostname: '127.0.0.1',
        port: hookHandle.port,
        path: `/api/hooks/${provider}`,
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': String(data.length),
          Authorization: `Bearer ${TOKEN}`,
        },
      },
      (res) => {
        res.on('data', () => {});
        res.on('end', () => resolve(res.statusCode ?? 0));
      },
    );
    req.on('error', reject);
    req.end(data);
  });
}

async function subscribe(client: ClientHandle, topics: string[]): Promise<void> {
  const req: Req = {
    kind: 'req',
    reqId: Math.floor(Math.random() * 1e9),
    method: 'subscribe',
    params: { topics },
  };
  client.sock.write(encodeNdjson(req));
  // Give the server a tick to apply the subscription update.
  await new Promise((r) => setImmediate(r));
}

describe('hook integration: HTTP → bridge → sink → mock client', () => {
  it('SessionStart → PreToolUse → PostToolUse → Stop fans out as agent.* topics', async () => {
    const client = await connectClient();
    await subscribe(client, ['agent:*']);

    const sessionId = 'sess-int-1';

    // 1. SessionStart registers a fresh agent id and emits agent.created.
    expect(
      await postHook('claude', {
        session_id: sessionId,
        hook_event_name: 'SessionStart',
        source: 'startup',
      }),
    ).toBe(200);
    const created = await client.waitFor((e) => e.topic === 'agent.created');
    expect(created.data.sessionId).toBe(sessionId);
    const agentId = created.data.id as number;
    expect(typeof agentId).toBe('number');

    // 2. PreToolUse → agent.toolStart, then statusChanged{active}.
    expect(
      await postHook('claude', {
        session_id: sessionId,
        hook_event_name: 'PreToolUse',
        tool_name: 'Write',
        tool_input: { file_path: '/tmp/foo.txt', content: 'hi' },
      }),
    ).toBe(200);
    const toolStart = await client.waitFor((e) => e.topic === 'agent.toolStart');
    expect(toolStart.data.id).toBe(agentId);
    expect(toolStart.data.toolName).toBe('Write');
    expect(toolStart.data.status).toBe('Writing foo.txt');
    const toolId = toolStart.data.toolId as string;
    expect(typeof toolId).toBe('string');

    // 3. PostToolUse pairs the same toolId via the bridge's stack.
    expect(
      await postHook('claude', { session_id: sessionId, hook_event_name: 'PostToolUse' }),
    ).toBe(200);
    const toolDone = await client.waitFor((e) => e.topic === 'agent.toolDone');
    expect(toolDone.data.id).toBe(agentId);
    expect(toolDone.data.toolId).toBe(toolId);

    // 4. Stop → idle status.
    expect(await postHook('claude', { session_id: sessionId, hook_event_name: 'Stop' })).toBe(200);
    const idle = await client.waitFor(
      (e) => e.topic === 'agent.statusChanged' && e.data.status === 'idle',
    );
    expect(idle.data.id).toBe(agentId);

    client.close();
  });

  it('rejects requests without the Bearer token', async () => {
    const status = await new Promise<number>((resolve, reject) => {
      const req = http.request(
        {
          hostname: '127.0.0.1',
          port: hookHandle.port,
          path: '/api/hooks/claude',
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
        },
        (res) => {
          res.on('data', () => {});
          res.on('end', () => resolve(res.statusCode ?? 0));
        },
      );
      req.on('error', reject);
      req.end('{}');
    });
    expect(status).toBe(401);
  });

  it('drops unknown providers without emitting any event', async () => {
    const client = await connectClient();
    await subscribe(client, ['agent:*']);
    // SessionStart routed via a bogus provider id — bridge ignores it.
    const status = await postHook('opencode', {
      session_id: 'sess-other',
      hook_event_name: 'SessionStart',
    });
    expect(status).toBe(200);
    // Give the server a brief window in case it does emit; assert nothing landed.
    await new Promise((r) => setTimeout(r, 100));
    expect(client.evts.filter((e) => e.topic.startsWith('agent.'))).toEqual([]);
    client.close();
  });
});
