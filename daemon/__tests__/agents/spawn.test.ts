import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { BroadcastSink } from '../../src/agents/broadcastSink.js';
import { LiveAgents } from '../../src/agents/liveAgents.js';
import { AgentsRegistry } from '../../src/agents/registry.js';
import { DaemonHookBridge } from '../../src/hookHost/bridge.js';
import { LayoutSaveDebouncer } from '../../src/layout/persistence.js';
import { createNullLogger } from '../../src/logging/logger.js';
import type { WriterTag } from '../../src/persistence/writerTag.js';
import { type ConnectionScope, type DispatchContext } from '../../src/rpc/dispatch.js';
import { buildMethodRegistry } from '../../src/rpc/methods/index.js';

/**
 * End-to-end through the agent.spawn / pty.input / agent.close RPC handlers.
 * Uses `/bin/cat` as the PTY child so we can drive stdin → stdout without
 * needing the real `claude` binary on PATH. Real node-pty under the hood.
 *
 * The handlers default to spawning `claude`; the test rebinds `agent.spawn`
 * to use a stub command before dispatching. Everything else (LiveAgents,
 * BroadcastSink, hookBridge, AgentsRegistry) is the real wiring.
 */

const OURS: WriterTag = { processId: process.pid, bootId: 'spawn-test' };

let tmpHome: string;
let originalHome: string | undefined;

function makeScope(): ConnectionScope {
  return {
    sessionId: 'sess-test',
    subscriptions: new Set(),
    sock: { destroyed: false, writable: true, write: () => true } as never,
  };
}

function makeCtx(): {
  ctx: DispatchContext;
  sink: BroadcastSink;
  recordedPty: Array<{ agentId: number; bytes: Buffer }>;
  liveAgents: LiveAgents;
  hookBridge: DaemonHookBridge;
} {
  const recordedPty: Array<{ agentId: number; bytes: Buffer }> = [];
  const sink = new BroadcastSink();
  (sink as unknown as { broadcastPty: (id: number, b: Buffer) => void }).broadcastPty = (
    id,
    bytes,
  ) => {
    recordedPty.push({ agentId: id, bytes });
  };
  const liveAgents = new LiveAgents();
  const logger = createNullLogger();
  const hookBridge = new DaemonHookBridge(sink, logger);
  const ctx: DispatchContext = {
    ours: OURS,
    sink,
    agents: new AgentsRegistry(OURS),
    layoutDebouncer: new LayoutSaveDebouncer(OURS, 10),
    liveAgents,
    hookBridge,
    logger,
    state: { layout: null, config: { externalAssetDirectories: [], logLevel: 'info' } },
    triggerShutdown: () => {},
  };
  return { ctx, sink, recordedPty, liveAgents, hookBridge };
}

/** Wait until `predicate` returns true, polling every 10ms; reject after `timeoutMs`. */
async function waitFor(predicate: () => boolean, timeoutMs = 3000): Promise<void> {
  const start = Date.now();
  while (!predicate()) {
    if (Date.now() - start > timeoutMs) throw new Error('waitFor timeout');
    await new Promise((r) => setTimeout(r, 10));
  }
}

beforeEach(() => {
  tmpHome = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-spawn-'));
  originalHome = process.env.PIXEL_AGENTS_HOME;
  process.env.PIXEL_AGENTS_HOME = tmpHome;
});

afterEach(() => {
  if (originalHome === undefined) delete process.env.PIXEL_AGENTS_HOME;
  else process.env.PIXEL_AGENTS_HOME = originalHome;
  try {
    fs.rmSync(tmpHome, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

// Skip on platforms without /bin/cat (Windows test runs aren't expected here
// since node-pty is the same OS-bound dep in production).
const HAS_CAT = fs.existsSync('/bin/cat');

describe.skipIf(!HAS_CAT)('agent.spawn end-to-end (real node-pty + /bin/cat)', () => {
  it('spawn → write → read → close round-trips', async () => {
    const { ctx, recordedPty, liveAgents, hookBridge } = makeCtx();
    const registry = buildMethodRegistry();

    // Override agent.spawn for this test so we don't depend on `claude` being
    // on PATH. /bin/cat echoes stdin to stdout — perfect for a round-trip
    // smoke. We re-implement the handler inline; production behaviour is
    // covered by the other unit tests.
    registry.register('agent.spawn.test', async (_p, _s, c) => {
      const { PtyHost } = await import('../../src/agents/ptyHost.js');
      const id = c.liveAgents.allocateId();
      const sessionId = `sess-${id}`;
      c.hookBridge.registerSession(sessionId, id);
      const pty = new PtyHost(
        {
          agentId: id,
          command: '/bin/cat',
          args: [],
          cwd: process.cwd(),
          logger: c.logger,
        },
        {
          onData: (bytes) => c.sink.broadcastPty(id, bytes),
          onExit: () => {
            c.liveAgents.remove(id);
          },
        },
      );
      c.liveAgents.add({ id, sessionId, cwd: process.cwd(), startedAt: Date.now(), pty });
      return { kind: 'res', reqId: 0, ok: true, data: { id, sessionId } };
    });

    const spawnRes = await registry.dispatch(1, 'agent.spawn.test', {}, makeScope(), ctx);
    expect(spawnRes.ok).toBe(true);
    if (!spawnRes.ok) return;
    const { id } = spawnRes.data as { id: number };
    expect(liveAgents.size()).toBe(1);
    expect(hookBridge.agentIdForSession(`sess-${id}`)).toBe(id);

    // pty.input handler is wired through the real registry.
    const payload = Buffer.from('echo-me\n').toString('base64');
    const inputRes = await registry.dispatch(
      2,
      'pty.input',
      { id, bytes: payload },
      makeScope(),
      ctx,
    );
    expect(inputRes.ok).toBe(true);

    // cat echoes the line; recordedPty captures it.
    await waitFor(() =>
      recordedPty.some((p) => p.agentId === id && p.bytes.toString('utf-8').includes('echo-me')),
    );

    // Close via agent.close (SIGTERM). The PTY exits, LiveAgents removes the entry.
    const closeRes = await registry.dispatch(3, 'agent.close', { id }, makeScope(), ctx);
    expect(closeRes.ok).toBe(true);
    await waitFor(() => liveAgents.size() === 0);
  });

  it('pty.resize on a live PTY succeeds', async () => {
    const { ctx, liveAgents } = makeCtx();
    const registry = buildMethodRegistry();
    const { PtyHost } = await import('../../src/agents/ptyHost.js');
    const id = ctx.liveAgents.allocateId();
    const sessionId = `sess-${id}`;
    ctx.hookBridge.registerSession(sessionId, id);
    const pty = new PtyHost(
      {
        agentId: id,
        command: '/bin/cat',
        args: [],
        cwd: process.cwd(),
        logger: ctx.logger,
      },
      { onData: () => {}, onExit: () => ctx.liveAgents.remove(id) },
    );
    ctx.liveAgents.add({ id, sessionId, cwd: process.cwd(), startedAt: Date.now(), pty });

    const res = await registry.dispatch(
      1,
      'pty.resize',
      { id, cols: 100, rows: 30 },
      makeScope(),
      ctx,
    );
    expect(res.ok).toBe(true);

    await registry.dispatch(2, 'agent.close', { id }, makeScope(), ctx);
    await waitFor(() => liveAgents.size() === 0);
  });
});
