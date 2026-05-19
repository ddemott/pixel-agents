import * as fs from 'fs';

import type { AgentEventSink } from '../../../src/messageSender.js';
import { cancelPermissionTimer, cancelWaitingTimer } from '../../../src/timerManager.js';
import { processTranscriptLine } from '../../../src/transcriptParser.js';
import type { AgentState } from '../../../src/types.js';
import type { Logger } from '../logging/logger.js';

const POLL_INTERVAL_MS = 500;
const MAX_READ_BYTES = 65536;

function makeAgentState(
  id: number,
  sessionId: string,
  cwd: string,
  jsonlFile: string,
  seedOffset: number,
): AgentState {
  return {
    id,
    sessionId,
    isExternal: false,
    projectDir: cwd,
    jsonlFile,
    fileOffset: seedOffset,
    lineBuffer: '',
    activeToolIds: new Set(),
    activeToolStatuses: new Map(),
    activeToolNames: new Map(),
    activeSubagentToolIds: new Map(),
    activeSubagentToolNames: new Map(),
    backgroundAgentToolIds: new Set(),
    isWaiting: false,
    permissionSent: false,
    hadToolsInTurn: false,
    lastDataAt: Date.now(),
    linesProcessed: 0,
    seenUnknownRecordTypes: new Set(),
    hookDelivered: false,
    inputTokens: 0,
    outputTokens: 0,
  };
}

/**
 * Polls each live agent's JSONL transcript at 500 ms intervals and calls
 * processTranscriptLine to emit agentToolStart / agentToolDone / agentStatus
 * events. Complements the DaemonHookBridge: hooks provide fast lifecycle
 * signals; JSONL provides rich tool content (status strings, sub-agent
 * tracking, token counts) that hooks don't carry.
 *
 * When hooks are active for an agent, hookDelivered suppresses duplicate
 * heuristic timers inside processTranscriptLine so the two streams don't race.
 */
export class JsonlPoller {
  private readonly agents = new Map<number, AgentState>();
  private readonly timers = new Map<number, ReturnType<typeof setInterval>>();
  private readonly waitingTimers = new Map<number, ReturnType<typeof setTimeout>>();
  private readonly permissionTimers = new Map<number, ReturnType<typeof setTimeout>>();

  constructor(
    private readonly sink: AgentEventSink,
    private readonly logger: Logger,
  ) {}

  /**
   * Start polling a JSONL file for `agentId`. seedOffset should be 0 for
   * freshly spawned agents (file doesn't exist yet) and `stat.size` for
   * revived agents (skip replaying full history on restart).
   */
  start(
    agentId: number,
    sessionId: string,
    cwd: string,
    jsonlPath: string,
    seedOffset: number,
  ): void {
    if (this.timers.has(agentId)) return; // already polling

    const state = makeAgentState(agentId, sessionId, cwd, jsonlPath, seedOffset);
    this.agents.set(agentId, state);

    const timer = setInterval(() => this.poll(agentId), POLL_INTERVAL_MS);
    timer.unref();
    this.timers.set(agentId, timer);
  }

  /** Stop polling and discard all state for `agentId`. */
  stop(agentId: number): void {
    const timer = this.timers.get(agentId);
    if (timer) {
      clearInterval(timer);
      this.timers.delete(agentId);
    }
    const wt = this.waitingTimers.get(agentId);
    if (wt) {
      clearTimeout(wt);
      this.waitingTimers.delete(agentId);
    }
    const pt = this.permissionTimers.get(agentId);
    if (pt) {
      clearTimeout(pt);
      this.permissionTimers.delete(agentId);
    }
    this.agents.delete(agentId);
  }

  /** Stop all pollers (called on daemon shutdown). */
  stopAll(): void {
    for (const id of [...this.timers.keys()]) this.stop(id);
  }

  /**
   * Mark an agent's hookDelivered flag so processTranscriptLine suppresses
   * heuristic timers. Call this when the hook bridge confirms a hook event was
   * delivered for the agent.
   */
  markHookDelivered(agentId: number): void {
    const agent = this.agents.get(agentId);
    if (agent) agent.hookDelivered = true;
  }

  private poll(agentId: number): void {
    const agent = this.agents.get(agentId);
    if (!agent) return;

    try {
      const stat = fs.statSync(agent.jsonlFile);
      if (stat.size <= agent.fileOffset) return;

      const bytesToRead = Math.min(stat.size - agent.fileOffset, MAX_READ_BYTES);
      const buf = Buffer.alloc(bytesToRead);
      const fd = fs.openSync(agent.jsonlFile, 'r');
      fs.readSync(fd, buf, 0, buf.length, agent.fileOffset);
      fs.closeSync(fd);
      agent.fileOffset += bytesToRead;

      const text = agent.lineBuffer + buf.toString('utf-8');
      const lines = text.split('\n');
      agent.lineBuffer = lines.pop() ?? '';

      const hasLines = lines.some((l) => l.trim());
      if (hasLines) {
        cancelWaitingTimer(agentId, this.waitingTimers);
        cancelPermissionTimer(agentId, this.permissionTimers);
        if (agent.permissionSent && !agent.hookDelivered && !agent.leadAgentId) {
          agent.permissionSent = false;
          this.sink.post({ type: 'agentToolPermissionClear', id: agentId });
        }
      }

      for (const line of lines) {
        if (!line.trim()) continue;
        processTranscriptLine(
          agentId,
          line,
          this.agents,
          this.waitingTimers,
          this.permissionTimers,
          this.sink,
        );
      }
    } catch (e) {
      if (e instanceof Error && 'code' in e && (e as NodeJS.ErrnoException).code === 'ENOENT') {
        return; // JSONL not created yet — normal for freshly spawned agents
      }
      this.logger.debug({ module: 'jsonlPoller', agentId, error: String(e) }, 'poll read error');
    }
  }
}
