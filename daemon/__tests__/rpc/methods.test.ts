import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { BroadcastSink } from '../../src/agents/broadcastSink.js';
import { AgentsRegistry, type PersistedAgent } from '../../src/agents/registry.js';
import type { PixelAgentsConfig } from '../../src/config/persistence.js';
import { LayoutSaveDebouncer } from '../../src/layout/persistence.js';
import type { WriterTag } from '../../src/persistence/writerTag.js';
import {
  type ConnectionScope,
  type DispatchContext,
  MethodRegistry,
} from '../../src/rpc/dispatch.js';
import { buildMethodRegistry } from '../../src/rpc/methods/index.js';

const OURS: WriterTag = { processId: 1, bootId: 'test-boot' };

let tmpDir: string;
let originalHome: string | undefined;

function makeScope(): ConnectionScope {
  return {
    sessionId: 'sess-1',
    subscriptions: new Set<string>(),
    // We never write to sock in these unit tests — the registry only touches
    // it indirectly via the BroadcastSink, which is also unused here.
    sock: { destroyed: false, writable: true, write: () => true } as never,
  };
}

async function makeCtx(): Promise<{
  ctx: DispatchContext;
  registry: MethodRegistry;
  recorded: Array<{ type: string; [k: string]: unknown }>;
}> {
  const recorded: Array<{ type: string; [k: string]: unknown }> = [];
  const sink = new BroadcastSink();
  // Replace `post` with a recorder so we can assert broadcasts without setting
  // up real sockets.
  (sink as unknown as { post: (e: { type: string }) => void }).post = (e) => {
    recorded.push(e as { type: string });
  };
  const agents = new AgentsRegistry(OURS);
  const ctx: DispatchContext = {
    ours: OURS,
    sink,
    agents,
    layoutDebouncer: new LayoutSaveDebouncer(OURS, 10),
    liveAgents: { get: () => undefined, list: () => [] } as never,
    hookBridge: { registerSession: () => {}, dropSession: () => {} } as never,
    logger: {
      trace: () => {},
      debug: () => {},
      info: () => {},
      warn: () => {},
      error: () => {},
      setLevel: () => {},
      close: () => {},
    },
    state: {
      layout: null,
      config: { externalAssetDirectories: [], logLevel: 'info' } as PixelAgentsConfig,
    },
    triggerShutdown: vi.fn(),
  };
  return { ctx, registry: buildMethodRegistry(), recorded };
}

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-methods-'));
  originalHome = process.env.HOME;
  process.env.HOME = tmpDir;
  vi.resetModules();
});

afterEach(() => {
  if (originalHome === undefined) delete process.env.HOME;
  else process.env.HOME = originalHome;
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

describe('layout methods', () => {
  it('layout.get returns the current in-memory layout', async () => {
    const { ctx, registry } = await makeCtx();
    ctx.state.layout = { version: 1, cols: 7 };
    const res = await registry.dispatch(1, 'layout.get', {}, makeScope(), ctx);
    expect(res.ok).toBe(true);
    if (res.ok) expect((res.data as { layout: { cols: number } }).layout.cols).toBe(7);
  });

  it('layout.save updates in-memory state and broadcasts layout.changed', async () => {
    const { ctx, registry, recorded } = await makeCtx();
    const layout = { version: 1, cols: 12 };
    const res = await registry.dispatch(2, 'layout.save', { layout }, makeScope(), ctx);
    expect(res.ok).toBe(true);
    expect(ctx.state.layout).toEqual(layout);
    expect(recorded[0].type).toBe('layout.changed');
    expect(recorded[0].source).toBe('client');
  });

  it('layout.save rejects missing layout param', async () => {
    const { ctx, registry } = await makeCtx();
    const res = await registry.dispatch(3, 'layout.save', {}, makeScope(), ctx);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error.code).toBe('bad_params');
  });

  it('layout.export errors when no layout loaded', async () => {
    const { ctx, registry } = await makeCtx();
    const res = await registry.dispatch(4, 'layout.export', {}, makeScope(), ctx);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error.code).toBe('no_layout');
  });

  it('layout.setDefault is gated as not_yet_supported', async () => {
    const { ctx, registry } = await makeCtx();
    const res = await registry.dispatch(5, 'layout.setDefault', {}, makeScope(), ctx);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error.code).toBe('not_yet_supported');
  });
});

describe('settings methods', () => {
  it('settings.get returns the current config', async () => {
    const { ctx, registry } = await makeCtx();
    ctx.state.config = { externalAssetDirectories: ['/a', '/b'], logLevel: 'info' };
    const res = await registry.dispatch(1, 'settings.get', {}, makeScope(), ctx);
    expect(res.ok).toBe(true);
    if (res.ok)
      expect(
        (res.data as { settings: PixelAgentsConfig }).settings.externalAssetDirectories,
      ).toEqual(['/a', '/b']);
  });

  it('settings.set applies a defensive patch and broadcasts', async () => {
    const { ctx, registry, recorded } = await makeCtx();
    const res = await registry.dispatch(
      2,
      'settings.set',
      { patch: { externalAssetDirectories: ['/x', 42, '/y'] } },
      makeScope(),
      ctx,
    );
    expect(res.ok).toBe(true);
    // 42 stripped by the array filter.
    expect(ctx.state.config.externalAssetDirectories).toEqual(['/x', '/y']);
    expect(recorded[0].type).toBe('settings.updated');
  });

  it('settings.set rejects missing patch', async () => {
    const { ctx, registry } = await makeCtx();
    const res = await registry.dispatch(3, 'settings.set', {}, makeScope(), ctx);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error.code).toBe('bad_params');
  });
});

describe('subscribe', () => {
  it('updates the scope topic filter', async () => {
    const { ctx, registry } = await makeCtx();
    const scope = makeScope();
    const res = await registry.dispatch(
      1,
      'subscribe',
      { topics: ['agent.toolStart', 'layout.changed'] },
      scope,
      ctx,
    );
    expect(res.ok).toBe(true);
    expect([...scope.subscriptions].sort()).toEqual(['agent.toolStart', 'layout.changed']);
  });

  it('rejects malformed topics', async () => {
    const { ctx, registry } = await makeCtx();
    const res = await registry.dispatch(2, 'subscribe', { topics: [1, 2] }, makeScope(), ctx);
    expect(res.ok).toBe(false);
  });
});

describe('daemon.shutdown', () => {
  it('triggers the shutdown callback asynchronously and returns ok', async () => {
    const { ctx, registry } = await makeCtx();
    const res = await registry.dispatch(1, 'daemon.shutdown', {}, makeScope(), ctx);
    expect(res.ok).toBe(true);
    // setImmediate defers — wait one turn of the event loop.
    await new Promise((r) => setImmediate(r));
    expect(ctx.triggerShutdown).toHaveBeenCalledTimes(1);
  });
});

describe('agent methods', () => {
  it('agent.list returns persisted agents for the given cwd', async () => {
    const { ctx, registry } = await makeCtx();
    const a: PersistedAgent = {
      id: 1,
      sessionId: 's1',
      palette: 0,
      hueShift: 0,
      lastSeenAt: Date.now(),
    };
    ctx.agents.upsert('/work', a);
    const res = await registry.dispatch(1, 'agent.list', { cwd: '/work' }, makeScope(), ctx);
    expect(res.ok).toBe(true);
    if (res.ok) expect((res.data as { agents: PersistedAgent[] }).agents).toHaveLength(1);
  });

  // Day 13-14 lit up agent.spawn/close + pty.input/resize. agent.focus and the
  // rest still wait for later phases.
  it.each(['agent.focus', 'agent.reassignSeat', 'agent.adopt', 'assets.list', 'hooks.toggle'])(
    '%s is gated as not_yet_supported',
    async (method) => {
      const { ctx, registry } = await makeCtx();
      const res = await registry.dispatch(1, method, {}, makeScope(), ctx);
      expect(res.ok).toBe(false);
      if (!res.ok) expect(res.error.code).toBe('not_yet_supported');
    },
  );

  it('agent.close returns not_found for unknown id (no live PTY)', async () => {
    const { ctx, registry } = await makeCtx();
    const res = await registry.dispatch(1, 'agent.close', { id: 999 }, makeScope(), ctx);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error.code).toBe('not_found');
  });

  it('pty.input rejects malformed params', async () => {
    const { ctx, registry } = await makeCtx();
    const res = await registry.dispatch(1, 'pty.input', { id: 1 }, makeScope(), ctx);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error.code).toBe('bad_params');
  });

  it('pty.resize rejects non-positive cols/rows', async () => {
    const { ctx, registry } = await makeCtx();
    const res = await registry.dispatch(
      1,
      'pty.resize',
      { id: 1, cols: 0, rows: 24 },
      makeScope(),
      ctx,
    );
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error.code).toBe('bad_params');
  });
});

describe('broadcast filtering', () => {
  it('respects per-connection subscriptions', () => {
    const sink = new BroadcastSink();
    type Recorder = { topics: string[] };
    const a: Recorder = { topics: [] };
    const b: Recorder = { topics: [] };
    const aSubs = new Set<string>(['layout.changed']);
    const bSubs = new Set<string>(); // unfiltered
    const mockSock = (rec: Recorder, _subs: Set<string>): unknown => ({
      destroyed: false,
      writable: true,
      write: (buf: Buffer) => {
        // Parse out the topic from the encoded NDJSON for the assertion.
        const nl = buf.indexOf(0x0a);
        const obj = JSON.parse(buf.slice(1, nl).toString('utf-8')) as { topic: string };
        rec.topics.push(obj.topic);
        return true;
      },
      once: () => {},
    });
    sink.register(mockSock(a, aSubs) as never, aSubs);
    sink.register(mockSock(b, bSubs) as never, bSubs);
    sink.post({ type: 'layout.changed', source: 'file', layout: null });
    sink.post({ type: 'agent.toolStart', id: 1 });
    expect(a.topics).toEqual(['layout.changed']);
    expect(b.topics).toEqual(['layout.changed', 'agent.toolStart']);
  });

  it('treats "*" subscription as wildcard', () => {
    const sink = new BroadcastSink();
    const got: string[] = [];
    const subs = new Set<string>(['*']);
    const sock: unknown = {
      destroyed: false,
      writable: true,
      write: (buf: Buffer) => {
        const nl = buf.indexOf(0x0a);
        const obj = JSON.parse(buf.slice(1, nl).toString('utf-8')) as { topic: string };
        got.push(obj.topic);
        return true;
      },
      once: () => {},
    };
    sink.register(sock as never, subs);
    sink.post({ type: 't1' });
    sink.post({ type: 't2' });
    expect(got).toEqual(['t1', 't2']);
  });
});
