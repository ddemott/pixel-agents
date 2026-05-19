import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import type { DaemonHookBridge } from '../hookHost/bridge.js';
import type { Logger } from '../logging/logger.js';
import type { BroadcastSink } from './broadcastSink.js';
import type { LiveAgents } from './liveAgents.js';
import { PtyHost, type SpawnFn } from './ptyHost.js';
import type { AgentsRegistry, PersistedAgent } from './registry.js';

const SESSION_STALE_MS = 30 * 24 * 60 * 60 * 1000; // 30 days
const RESUME_HEALTH_MS = 3000;
const DEFAULT_COLS = 120;
const DEFAULT_ROWS = 40;
const EARLY_BUF_LIMIT = 4096;
// Matches the exact string claude emits when the session JSONL format is
// incompatible with the running binary version. Must stay in sync if claude
// ever changes its error message.
const VERSION_MISMATCH_STR = 'session format version mismatch';

export interface ReviveContext {
  agents: AgentsRegistry;
  liveAgents: LiveAgents;
  sink: BroadcastSink;
  hookBridge: DaemonHookBridge;
  logger: Logger;
  /** Test seam: override the PTY spawn function used by PtyHost. */
  spawnOverride?: SpawnFn;
  /** Test seam: override the 3 s health timeout (default: 3000). */
  healthTimeoutMs?: number;
}

/**
 * Compute where Claude stores the JSONL transcript for a given cwd + sessionId.
 * Formula: `~/.claude/projects/<cwd-normalized>/<sessionId>.jsonl`
 * where normalization replaces every non-alphanumeric-non-dash char with `-`.
 */
export function resolveJsonlPath(cwd: string, sessionId: string): string {
  const dirName = cwd.replace(/[^a-zA-Z0-9-]/g, '-');
  return path.join(os.homedir(), '.claude', 'projects', dirName, `${sessionId}.jsonl`);
}

/**
 * On daemon boot: iterate agents.json, run a JSONL liveness gate, spawn
 * `claude --resume <sessionId>` for each surviving entry, then run a 3 s
 * health check to detect immediate failures.
 *
 * Failure paths (arch §16):
 *
 * | Condition                          | Detection                      | Action              |
 * | JSONL missing                      | fs.existsSync                  | Drop entry + log    |
 * | JSONL stale (>30 days)             | mtime check                    | Drop entry + log    |
 * | claude binary missing              | exit code 127                  | Keep; spawnFailed   |
 * | claude --resume unknown session    | exit code ≠ 0 within 3 s      | Drop entry + log    |
 * | claude version mismatch            | exit code 2 + stderr text      | Keep; spawnFailed   |
 * | claude hangs (network/auth)        | 3 s timeout → no exit          | Keep PTY alive      |
 */
export async function reviveAgentsOnBoot(ctx: ReviveContext): Promise<void> {
  const healthMs = ctx.healthTimeoutMs ?? RESUME_HEALTH_MS;
  for (const cwd of ctx.agents.cwds()) {
    for (const entry of ctx.agents.forCwd(cwd)) {
      try {
        await reviveOne(cwd, entry, ctx, healthMs);
      } catch (e) {
        const reason = e instanceof Error ? e.message : String(e);
        ctx.logger.info(
          { module: 'resume', agentId: entry.id, sessionId: entry.sessionId, reason },
          'revival failed — dropping entry',
        );
        ctx.agents.remove(cwd, entry.id);
      }
    }
  }
}

async function reviveOne(
  cwd: string,
  entry: PersistedAgent,
  ctx: ReviveContext,
  healthMs: number,
): Promise<void> {
  // ── JSONL liveness gate ──────────────────────────────────────────────────
  const jPath = resolveJsonlPath(cwd, entry.sessionId);
  if (!fs.existsSync(jPath)) throw new Error('jsonl missing');
  const mtime = fs.statSync(jPath).mtimeMs;
  if (Date.now() - mtime > SESSION_STALE_MS) throw new Error('jsonl stale (>30 days)');

  // Reserve the id before spawning so allocateId() never collides with it.
  ctx.liveAgents.reserveId(entry.id);
  // Pre-register so inbound SessionStart hook reuses this id.
  ctx.hookBridge.registerSession(entry.sessionId, entry.id);

  // ── Spawn claude --resume ────────────────────────────────────────────────
  let earlyBuf = '';
  // Resolved by onExit inside PtyHost; races against the health timeout below.
  let notifyExit!: (info: { exitCode: number; signal?: number }) => void;
  const exitEvent = new Promise<{ exitCode: number; signal?: number }>((r) => (notifyExit = r));

  let pty: PtyHost;
  try {
    pty = new PtyHost(
      {
        agentId: entry.id,
        command: 'claude',
        args: ['--resume', entry.sessionId],
        cwd,
        cols: DEFAULT_COLS,
        rows: DEFAULT_ROWS,
        logger: ctx.logger,
        spawn: ctx.spawnOverride,
      },
      {
        onData: (bytes) => {
          if (earlyBuf.length < EARLY_BUF_LIMIT) earlyBuf += bytes.toString('utf-8');
          ctx.sink.broadcastPty(entry.id, bytes);
        },
        onExit: (exitCode, signal) => {
          notifyExit({ exitCode, signal });
          ctx.liveAgents.remove(entry.id);
          ctx.sink.emitTo(entry.id, {
            type: 'agent.exited',
            id: entry.id,
            exitCode,
            signal,
            reason: classifyExit(exitCode, signal, earlyBuf),
          });
          ctx.logger.info(
            { module: 'resume', agentId: entry.id, exitCode, signal },
            'revived pty exited',
          );
        },
      },
    );
  } catch (e) {
    // Synchronous ENOENT — guard for environments where node-pty throws on
    // spawn rather than surfacing the failure through onExit.
    ctx.hookBridge.dropSession(entry.sessionId);
    ctx.sink.post({
      type: 'agent.spawnFailed',
      id: entry.id,
      sessionId: entry.sessionId,
      reason: 'claude_missing',
    });
    return; // Keep entry in agents.json — binary may be installed later.
  }

  ctx.liveAgents.add({ id: entry.id, sessionId: entry.sessionId, cwd, startedAt: Date.now(), pty });

  // ── 3 s health check ─────────────────────────────────────────────────────
  const healthDone = new Promise<null>((r) => setTimeout(() => r(null), healthMs));
  const winner = await Promise.race([exitEvent, healthDone]);

  if (winner === null) {
    // Survived the health window — session is live.
    ctx.agents.upsert(cwd, { ...entry, lastSeenAt: Date.now() });
    ctx.sink.emitTo(entry.id, {
      type: 'agent.created',
      id: entry.id,
      sessionId: entry.sessionId,
      cwd,
      palette: entry.palette,
      hueShift: entry.hueShift,
      seatId: entry.seatId,
      isResumed: true,
    });
    return;
  }

  // Exited within the health window — classify the cause.
  const { exitCode } = winner;

  if (exitCode === 127) {
    // Shell could not exec claude — binary not on PATH.
    ctx.sink.post({
      type: 'agent.spawnFailed',
      id: entry.id,
      sessionId: entry.sessionId,
      reason: 'claude_missing',
    });
    return; // Keep entry — binary may be installed later.
  }

  if (exitCode === 2 && earlyBuf.includes(VERSION_MISMATCH_STR)) {
    // Claude upgraded; session format incompatible with current binary.
    ctx.sink.post({
      type: 'agent.spawnFailed',
      id: entry.id,
      sessionId: entry.sessionId,
      reason: 'claude_upgraded',
    });
    return; // Keep entry — user can resume manually after upgrade.
  }

  // Any other early exit → session unrecognized or already expired; drop it.
  throw new Error(`claude --resume exited early (code ${exitCode ?? 'signal'})`);
}

/**
 * Classify an agent exit into a semantic reason string.
 * Used both for revived agents (onExit in reviveOne) and exposed for
 * consistent use in `agents.ts` normal-spawn onExit handlers.
 */
export function classifyExit(
  exitCode: number,
  signal: number | undefined,
  earlyOutput = '',
): string {
  if (exitCode === 0) return 'user';
  if (exitCode === 127) return 'claude_missing';
  if (exitCode === 2 && earlyOutput.includes(VERSION_MISMATCH_STR)) return 'claude_upgraded';
  if (signal !== undefined) return 'crash';
  return 'crash';
}
