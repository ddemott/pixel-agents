import * as crypto from 'crypto';

import { PtyHost } from '../../agents/ptyHost.js';
import type { PersistedAgent } from '../../agents/registry.js';
import { classifyExit } from '../../agents/resume.js';
import { err, type Handler, type MethodRegistry, ok } from '../dispatch.js';

const DEFAULT_COLS = 120;
const DEFAULT_ROWS = 40;
const KILL_GRACE_MS = 2000;

interface ListParams {
  cwd?: string;
}

function isListParams(p: unknown): p is ListParams {
  if (p === null || p === undefined) return true;
  if (typeof p !== 'object' || Array.isArray(p)) return false;
  const cwd = (p as ListParams).cwd;
  return cwd === undefined || typeof cwd === 'string';
}

interface SpawnParams {
  cwd?: string;
  bypassPermissions?: boolean;
  cols?: number;
  rows?: number;
}

function isSpawnParams(p: unknown): p is SpawnParams {
  if (p === null || p === undefined) return true;
  if (typeof p !== 'object' || Array.isArray(p)) return false;
  const x = p as SpawnParams;
  if (x.cwd !== undefined && typeof x.cwd !== 'string') return false;
  if (x.bypassPermissions !== undefined && typeof x.bypassPermissions !== 'boolean') return false;
  if (x.cols !== undefined && typeof x.cols !== 'number') return false;
  if (x.rows !== undefined && typeof x.rows !== 'number') return false;
  return true;
}

interface IdParams {
  id: number;
}

function isIdParams(p: unknown): p is IdParams {
  return typeof p === 'object' && p !== null && typeof (p as IdParams).id === 'number';
}

interface PtyInputParams {
  id: number;
  bytes: string; // base64
}

function isPtyInputParams(p: unknown): p is PtyInputParams {
  return (
    typeof p === 'object' &&
    p !== null &&
    typeof (p as PtyInputParams).id === 'number' &&
    typeof (p as PtyInputParams).bytes === 'string'
  );
}

interface PtyResizeParams {
  id: number;
  cols: number;
  rows: number;
}

function isPtyResizeParams(p: unknown): p is PtyResizeParams {
  return (
    typeof p === 'object' &&
    p !== null &&
    typeof (p as PtyResizeParams).id === 'number' &&
    typeof (p as PtyResizeParams).cols === 'number' &&
    typeof (p as PtyResizeParams).rows === 'number'
  );
}

/** Stubs whose impl still lives in later phases. */
const NOT_YET: Record<string, string> = {
  'agent.focus': 'agent focus lands in Phase 2 Day 6 (focus arbitration)',
  'agent.reassignSeat': 'seat reassignment lands once the office editor wires through',
  'agent.adopt': 'external session adoption lands once JSONL polling is fully ported',
  'pty.resync': 'PTY resync lands in Phase 2 (scrollback snapshot)',
  'assets.list': 'asset loader port lands after persistence',
  'assets.requestBlob': 'asset blob streaming lands in Phase 3',
  'assets.addDir': 'asset directory management lands once asset loader ships',
  'assets.removeDir': 'asset directory management lands once asset loader ships',
  'hooks.toggle': 'hook toggle RPC lands once persistence covers hook settings',
};

function notYetHandler(method: string): Handler {
  const reason = NOT_YET[method] ?? `not yet supported: ${method}`;
  return () => err('not_yet_supported', reason);
}

export function registerAgentMethods(reg: MethodRegistry): void {
  reg.register('agent.list', (params, _s, ctx) => {
    if (!isListParams(params)) {
      return err('bad_params', 'agent.list expects { cwd?: string } or no params');
    }
    const cwd = params?.cwd ?? process.cwd();
    return ok({ agents: ctx.agents.forCwd(cwd) });
  });

  reg.register('agent.spawn', (params, _s, ctx) => {
    if (!isSpawnParams(params)) {
      return err('bad_params', 'agent.spawn expects { cwd?, bypassPermissions?, cols?, rows? }');
    }
    const cwd = params?.cwd ?? process.cwd();
    const cols = params?.cols ?? DEFAULT_COLS;
    const rows = params?.rows ?? DEFAULT_ROWS;
    const bypassPermissions = params?.bypassPermissions === true;

    const sessionId = crypto.randomUUID();
    const agentId = ctx.liveAgents.allocateId();

    // Pre-register the session→agentId mapping so the inbound SessionStart
    // hook from claude reuses this id instead of allocating a fresh one.
    ctx.hookBridge.registerSession(sessionId, agentId);

    const command = 'claude';
    const args = ['--session-id', sessionId];
    if (bypassPermissions) args.push('--dangerously-skip-permissions');

    let pty: PtyHost;
    try {
      pty = new PtyHost(
        { agentId, command, args, cwd, cols, rows, logger: ctx.logger },
        {
          onData: (bytes) => ctx.sink.broadcastPty(agentId, bytes),
          onExit: (exitCode, signal) => {
            ctx.liveAgents.remove(agentId);
            const reason = classifyExit(exitCode, signal);
            ctx.sink.emitTo(agentId, {
              type: 'agent.exited',
              id: agentId,
              exitCode,
              signal,
              reason,
            });
            // Clean exits (user-initiated /exit) remove from persistence so
            // the next daemon boot doesn't attempt --resume for a closed session.
            // Crashes keep the entry so --resume can revive on restart.
            if (reason === 'user') ctx.agents.remove(cwd, agentId);
            ctx.logger.info(
              { module: 'agentSpawn', agentId, exitCode, signal, reason },
              'pty exited',
            );
          },
        },
      );
    } catch (e) {
      ctx.hookBridge.dropSession(sessionId);
      const message = e instanceof Error ? e.message : String(e);
      return err('spawn_failed', `claude spawn failed: ${message}`);
    }

    ctx.liveAgents.add({
      id: agentId,
      sessionId,
      cwd,
      startedAt: Date.now(),
      pty,
    });

    const persisted: PersistedAgent = {
      id: agentId,
      sessionId,
      palette: 0,
      hueShift: 0,
      lastSeenAt: Date.now(),
    };
    ctx.agents.upsert(cwd, persisted);

    ctx.sink.emitTo(agentId, {
      type: 'agent.created',
      id: agentId,
      sessionId,
      cwd,
      palette: persisted.palette,
      hueShift: persisted.hueShift,
    });

    return ok({ id: agentId, sessionId });
  });

  reg.register('agent.close', (params, _s, ctx) => {
    if (!isIdParams(params)) {
      return err('bad_params', 'agent.close expects { id: number }');
    }
    const live = ctx.liveAgents.get(params.id);
    if (!live) return err('not_found', `no live agent with id ${params.id}`);

    live.pty.kill('SIGTERM');
    // Escalate to SIGKILL if the child hasn't exited within the grace window.
    const escalate = setTimeout(() => {
      if (ctx.liveAgents.get(params.id)) live.pty.kill('SIGKILL');
    }, KILL_GRACE_MS);
    escalate.unref();

    ctx.agents.remove(live.cwd, live.id);
    return ok({});
  });

  reg.register('pty.input', (params, _s, ctx) => {
    if (!isPtyInputParams(params)) {
      return err('bad_params', 'pty.input expects { id: number, bytes: base64 }');
    }
    const live = ctx.liveAgents.get(params.id);
    if (!live) return err('not_found', `no live agent with id ${params.id}`);
    let buf: Buffer;
    try {
      buf = Buffer.from(params.bytes, 'base64');
    } catch {
      return err('bad_params', 'pty.input bytes must be valid base64');
    }
    live.pty.write(buf);
    return ok({});
  });

  reg.register('pty.resize', (params, _s, ctx) => {
    if (!isPtyResizeParams(params)) {
      return err('bad_params', 'pty.resize expects { id, cols, rows }');
    }
    if (params.cols < 1 || params.rows < 1) {
      return err('bad_params', 'cols and rows must be >= 1');
    }
    const live = ctx.liveAgents.get(params.id);
    if (!live) return err('not_found', `no live agent with id ${params.id}`);
    live.pty.resize(params.cols, params.rows);
    return ok({});
  });

  for (const method of Object.keys(NOT_YET)) {
    reg.register(method, notYetHandler(method));
  }
}
