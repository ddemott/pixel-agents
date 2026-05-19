import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PtyHandle } from '../../src/agents/ptyHost.js';

/**
 * vi.resetModules() + dynamic imports isolate each test from shared
 * AGENTS_JSON_PATH state, matching the pattern in agentsRegistry.test.ts.
 * PIXEL_AGENTS_DIR is a module-level const in paths.ts derived from
 * os.homedir(); overriding HOME before a reset forces a fresh derivation.
 */

const SESSION_ID = 'aaaaaaaa-0000-0000-0000-000000000001';
const CWD = process.cwd();

let tmpHome: string;
let origHome: string | undefined;

beforeEach(() => {
  tmpHome = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-resume-'));
  origHome = process.env.HOME;
  process.env.HOME = tmpHome;
  vi.resetModules();
});

afterEach(() => {
  if (origHome !== undefined) process.env.HOME = origHome;
  try {
    fs.rmSync(tmpHome, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

type WriterTag = { processId: number; bootId: string };
const OURS: WriterTag = { processId: process.pid, bootId: 'resume-test' };

/** Write a dummy JSONL file at the path resolveJsonlPath would compute. */
async function makeJsonl(cwd: string, sessionId: string, mtime?: Date): Promise<string> {
  const { resolveJsonlPath } = await import('../../src/agents/resume.js');
  const p = resolveJsonlPath(cwd, sessionId);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, '{"type":"system"}\n');
  if (mtime) fs.utimesSync(p, mtime, mtime);
  return p;
}

/**
 * Build a fake PtyHandle.
 *
 * @param exitCode  Exit code to deliver via setImmediate. Omit → never exits
 *                  (simulates a process that survives the health window).
 * @param output    Optional data to emit before the exit event.
 */
function makeFakeHandle(exitCode?: number, output?: string): PtyHandle {
  let dataCb: ((d: string | Buffer) => void) | null = null;
  let exitCb: ((e: { exitCode: number; signal?: number }) => void) | null = null;

  const handle: PtyHandle = {
    pid: 99999,
    onData(cb) {
      dataCb = cb;
      return { dispose() {} };
    },
    onExit(cb) {
      exitCb = cb;
      if (exitCode !== undefined) {
        setImmediate(() => {
          if (output) dataCb?.(output);
          exitCb?.({ exitCode });
        });
      }
      return { dispose() {} };
    },
    write() {},
    resize() {},
    kill() {},
  };
  return handle;
}

async function makeCtx(spawnOverride?: () => PtyHandle) {
  const [
    { BroadcastSink },
    { LiveAgents },
    { AgentsRegistry },
    { DaemonHookBridge },
    { createNullLogger },
  ] = await Promise.all([
    import('../../src/agents/broadcastSink.js'),
    import('../../src/agents/liveAgents.js'),
    import('../../src/agents/registry.js'),
    import('../../src/hookHost/bridge.js'),
    import('../../src/logging/logger.js'),
  ]);

  const sink = new BroadcastSink();
  const liveAgents = new LiveAgents();
  const agents = new AgentsRegistry(OURS);
  const logger = createNullLogger();
  const hookBridge = new DaemonHookBridge(sink, logger);

  const { reviveAgentsOnBoot } = await import('../../src/agents/resume.js');

  return {
    ctx: {
      agents,
      liveAgents,
      sink,
      hookBridge,
      logger,
      spawnOverride: spawnOverride
        ? (_cmd: string, _args: string[], _opts: unknown) => spawnOverride()
        : undefined,
      healthTimeoutMs: 150,
    },
    agents,
    liveAgents,
    sink,
    reviveAgentsOnBoot,
  };
}

describe('reviveAgentsOnBoot', () => {
  it('drops entry when JSONL file is missing', async () => {
    const { ctx, agents, reviveAgentsOnBoot } = await makeCtx();
    agents.upsert(CWD, {
      id: 1,
      sessionId: SESSION_ID,
      palette: 0,
      hueShift: 0,
      lastSeenAt: Date.now(),
    });

    await reviveAgentsOnBoot(ctx);

    expect(agents.forCwd(CWD).find((e) => e.id === 1)).toBeUndefined();
  });

  it('drops entry when JSONL is stale (>30 days)', async () => {
    const { ctx, agents, reviveAgentsOnBoot } = await makeCtx();
    const staleDate = new Date(Date.now() - 31 * 24 * 60 * 60 * 1000);
    await makeJsonl(CWD, SESSION_ID, staleDate);
    agents.upsert(CWD, {
      id: 1,
      sessionId: SESSION_ID,
      palette: 0,
      hueShift: 0,
      lastSeenAt: Date.now(),
    });

    await reviveAgentsOnBoot(ctx);

    expect(agents.forCwd(CWD).find((e) => e.id === 1)).toBeUndefined();
  });

  it('emits agent.created { isResumed: true } when PTY survives health window', async () => {
    const handle = makeFakeHandle(/* never exits */);
    const { ctx, agents, liveAgents, sink, reviveAgentsOnBoot } = await makeCtx(() => handle);
    await makeJsonl(CWD, SESSION_ID);
    agents.upsert(CWD, {
      id: 5,
      sessionId: SESSION_ID,
      palette: 2,
      hueShift: 90,
      lastSeenAt: Date.now() - 1000,
    });

    const emitted: unknown[] = [];
    const orig = sink.emitTo.bind(sink);
    sink.emitTo = (id, evt) => {
      emitted.push(evt);
      orig(id, evt);
    };

    await reviveAgentsOnBoot(ctx);

    expect(liveAgents.size()).toBe(1);
    const created = emitted.find((e) => (e as { type: string }).type === 'agent.created');
    expect(created).toBeDefined();
    const evt = created as { isResumed: boolean; id: number; palette: number; hueShift: number };
    expect(evt.isResumed).toBe(true);
    expect(evt.id).toBe(5);
    expect(evt.palette).toBe(2);
    expect(evt.hueShift).toBe(90);
    expect(agents.forCwd(CWD).find((e) => e.id === 5)).toBeDefined();
    handle.kill();
  });

  it('refreshes lastSeenAt on successful revival', async () => {
    const handle = makeFakeHandle();
    const { ctx, agents, reviveAgentsOnBoot } = await makeCtx(() => handle);
    const oldTs = Date.now() - 5000;
    await makeJsonl(CWD, SESSION_ID);
    agents.upsert(CWD, {
      id: 7,
      sessionId: SESSION_ID,
      palette: 0,
      hueShift: 0,
      lastSeenAt: oldTs,
    });

    const before = Date.now();
    await reviveAgentsOnBoot(ctx);

    const entry = agents.forCwd(CWD).find((e) => e.id === 7);
    expect(entry?.lastSeenAt).toBeGreaterThanOrEqual(before);
    handle.kill();
  });

  it('emits agent.spawnFailed { reason: "claude_missing" } on exit 127, keeps entry', async () => {
    const handle = makeFakeHandle(127);
    const { ctx, agents, liveAgents, sink, reviveAgentsOnBoot } = await makeCtx(() => handle);
    await makeJsonl(CWD, SESSION_ID);
    agents.upsert(CWD, {
      id: 2,
      sessionId: SESSION_ID,
      palette: 0,
      hueShift: 0,
      lastSeenAt: Date.now(),
    });

    const posted: unknown[] = [];
    const origPost = sink.post.bind(sink);
    sink.post = (evt) => {
      posted.push(evt);
      origPost(evt);
    };

    await reviveAgentsOnBoot(ctx);

    expect(agents.forCwd(CWD).find((e) => e.id === 2)).toBeDefined(); // kept
    expect(liveAgents.size()).toBe(0); // PTY already exited
    const failed = posted.find((e) => (e as { type: string }).type === 'agent.spawnFailed');
    expect(failed).toBeDefined();
    expect((failed as { reason: string }).reason).toBe('claude_missing');
  });

  it('emits agent.spawnFailed { reason: "claude_upgraded" } on exit 2 + version mismatch, keeps entry', async () => {
    const handle = makeFakeHandle(2, 'Error: session format version mismatch');
    const { ctx, agents, liveAgents, sink, reviveAgentsOnBoot } = await makeCtx(() => handle);
    await makeJsonl(CWD, SESSION_ID);
    agents.upsert(CWD, {
      id: 3,
      sessionId: SESSION_ID,
      palette: 0,
      hueShift: 0,
      lastSeenAt: Date.now(),
    });

    const posted: unknown[] = [];
    const origPost = sink.post.bind(sink);
    sink.post = (evt) => {
      posted.push(evt);
      origPost(evt);
    };

    await reviveAgentsOnBoot(ctx);

    expect(agents.forCwd(CWD).find((e) => e.id === 3)).toBeDefined(); // kept
    expect(liveAgents.size()).toBe(0);
    const failed = posted.find((e) => (e as { type: string }).type === 'agent.spawnFailed');
    expect((failed as { reason: string }).reason).toBe('claude_upgraded');
  });

  it('drops entry on any other early exit (unknown session)', async () => {
    const handle = makeFakeHandle(1);
    const { ctx, agents, liveAgents, reviveAgentsOnBoot } = await makeCtx(() => handle);
    await makeJsonl(CWD, SESSION_ID);
    agents.upsert(CWD, {
      id: 4,
      sessionId: SESSION_ID,
      palette: 0,
      hueShift: 0,
      lastSeenAt: Date.now(),
    });

    await reviveAgentsOnBoot(ctx);

    expect(agents.forCwd(CWD).find((e) => e.id === 4)).toBeUndefined(); // dropped
    expect(liveAgents.size()).toBe(0);
  });

  it('revives multiple agents across cwds independently', async () => {
    const SESSION2 = 'aaaaaaaa-0000-0000-0000-000000000002';
    const CWD2 = path.join(os.tmpdir(), 'other-project');
    let spawnCount = 0;
    const h1 = makeFakeHandle();
    const h2 = makeFakeHandle();
    const { ctx, liveAgents, reviveAgentsOnBoot } = await makeCtx(() =>
      spawnCount++ === 0 ? h1 : h2,
    );

    await makeJsonl(CWD, SESSION_ID);
    await makeJsonl(CWD2, SESSION2);
    const { agents } = ctx;
    agents.upsert(CWD, {
      id: 10,
      sessionId: SESSION_ID,
      palette: 0,
      hueShift: 0,
      lastSeenAt: Date.now(),
    });
    agents.upsert(CWD2, {
      id: 11,
      sessionId: SESSION2,
      palette: 1,
      hueShift: 0,
      lastSeenAt: Date.now(),
    });

    await reviveAgentsOnBoot(ctx);

    expect(liveAgents.size()).toBe(2);
    h1.kill();
    h2.kill();
  });

  it('reserves ids so future allocateId() does not collide with revived ids', async () => {
    const handle = makeFakeHandle();
    const { ctx, liveAgents, reviveAgentsOnBoot } = await makeCtx(() => handle);
    await makeJsonl(CWD, SESSION_ID);
    ctx.agents.upsert(CWD, {
      id: 20,
      sessionId: SESSION_ID,
      palette: 0,
      hueShift: 0,
      lastSeenAt: Date.now(),
    });

    await reviveAgentsOnBoot(ctx);

    expect(liveAgents.allocateId()).toBeGreaterThan(20);
    handle.kill();
  });

  it('resolveJsonlPath builds the expected path', async () => {
    const { resolveJsonlPath } = await import('../../src/agents/resume.js');
    const p = resolveJsonlPath('/home/user/my project', 'sess-abc');
    expect(p).toContain('-home-user-my-project');
    expect(p).toMatch(/sess-abc\.jsonl$/);
  });
});
