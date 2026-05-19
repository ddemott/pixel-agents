import * as path from 'path';

import type { AgentEvent, AgentEventSink } from '../../../src/messageSender.js';
import type { Logger } from '../logging/logger.js';

/**
 * Translates raw provider hook payloads into daemon RPC events on the broadcast
 * sink. Owns the `sessionId → agentId` map: SessionStart for an unknown session
 * allocates a new agent id and emits `agent.created`; subsequent PreToolUse /
 * PostToolUse / Stop events are routed by session id and emitted with the
 * matching `agent.*` topic. Stays minimal in Day 12 — full lifecycle (palette
 * assignment, seat picking, persistence) lands with agent.spawn in Day 13-14.
 *
 * Unknown providers are dropped; unknown event names are dropped after a debug
 * log line so we don't quietly swallow new payloads. Bridge is sink-bound: the
 * BroadcastSink filters per-agent subscribers, so we always `emitTo(agentId, …)`
 * even when the topic is global.
 */
export class DaemonHookBridge {
  private readonly sessionToAgentId = new Map<string, number>();
  private nextAgentId = 1;
  /** Per-agent stack of active toolIds so a bare PostToolUse can be paired. */
  private readonly toolStack = new Map<number, string[]>();

  constructor(
    private readonly sink: AgentEventSink,
    private readonly logger: Logger,
  ) {}

  /** Look up the agent id for a known session, or `undefined` if unregistered. */
  agentIdForSession(sessionId: string): number | undefined {
    return this.sessionToAgentId.get(sessionId);
  }

  /** Test seam: pre-seed a session→agent mapping (skipping SessionStart). */
  registerSession(sessionId: string, agentId: number): void {
    this.sessionToAgentId.set(sessionId, agentId);
    if (agentId >= this.nextAgentId) this.nextAgentId = agentId + 1;
  }

  /** Entry point invoked by the hook HTTP server. */
  handleEvent(providerId: string, raw: Record<string, unknown>): void {
    if (providerId !== 'claude') {
      this.logger.debug({ module: 'hookBridge', providerId }, 'unknown provider, dropping event');
      return;
    }
    const eventName = raw.hook_event_name;
    const sessionId = raw.session_id;
    if (typeof eventName !== 'string' || typeof sessionId !== 'string') return;

    switch (eventName) {
      case 'SessionStart':
        this.onSessionStart(sessionId, raw);
        return;
      case 'SessionEnd':
        this.onSessionEnd(sessionId, raw);
        return;
      case 'UserPromptSubmit':
        this.emitStatus(sessionId, 'active');
        return;
      case 'PreToolUse':
        this.onPreToolUse(sessionId, raw);
        return;
      case 'PostToolUse':
      case 'PostToolUseFailure':
        this.onPostToolUse(sessionId);
        return;
      case 'Stop':
        this.emitStatus(sessionId, 'idle');
        return;
      case 'PermissionRequest':
        this.emitStatus(sessionId, 'permission');
        return;
      case 'Notification': {
        const t = raw.notification_type;
        if (t === 'permission_prompt') this.emitStatus(sessionId, 'permission');
        else if (t === 'idle_prompt') this.emitStatus(sessionId, 'idle');
        return;
      }
      default:
        this.logger.debug({ module: 'hookBridge', eventName, sessionId }, 'unhandled hook event');
    }
  }

  private onSessionStart(sessionId: string, raw: Record<string, unknown>): void {
    const existing = this.sessionToAgentId.get(sessionId);
    if (existing !== undefined) {
      // Resume/clear flows reuse the existing agent id; just refresh status.
      this.emitStatus(sessionId, 'active');
      return;
    }
    const agentId = this.nextAgentId++;
    this.sessionToAgentId.set(sessionId, agentId);
    const source = typeof raw.source === 'string' ? raw.source : 'startup';
    this.emit(agentId, {
      type: 'agent.created',
      id: agentId,
      sessionId,
      source,
    });
    // Most SessionStart payloads coincide with an active user turn; emit a
    // status event so subscribers don't have to special-case the first frame.
    this.emit(agentId, {
      type: 'agent.statusChanged',
      id: agentId,
      status: 'active',
    });
  }

  private onSessionEnd(sessionId: string, raw: Record<string, unknown>): void {
    const agentId = this.sessionToAgentId.get(sessionId);
    if (agentId === undefined) return;
    const reason = typeof raw.reason === 'string' ? raw.reason : 'exit';
    // `clear` and `resume` are followed by a SessionStart within ms — keep the
    // mapping so the next event reuses the same agent id.
    if (reason !== 'clear' && reason !== 'resume') {
      this.sessionToAgentId.delete(sessionId);
      this.toolStack.delete(agentId);
    }
    this.emit(agentId, { type: 'agent.exited', id: agentId, reason });
  }

  private onPreToolUse(sessionId: string, raw: Record<string, unknown>): void {
    const agentId = this.sessionToAgentId.get(sessionId);
    if (agentId === undefined) return;
    const toolName = typeof raw.tool_name === 'string' ? raw.tool_name : 'unknown';
    const toolInput =
      typeof raw.tool_input === 'object' && raw.tool_input !== null
        ? (raw.tool_input as Record<string, unknown>)
        : {};
    const toolId = `hook-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    this.pushTool(agentId, toolId);
    this.emit(agentId, {
      type: 'agent.toolStart',
      id: agentId,
      toolId,
      toolName,
      status: shortStatus(toolName, toolInput),
    });
    this.emit(agentId, {
      type: 'agent.statusChanged',
      id: agentId,
      status: 'active',
    });
  }

  private onPostToolUse(sessionId: string): void {
    const agentId = this.sessionToAgentId.get(sessionId);
    if (agentId === undefined) return;
    const toolId = this.popTool(agentId);
    if (!toolId) return;
    this.emit(agentId, { type: 'agent.toolDone', id: agentId, toolId });
  }

  private emitStatus(
    sessionId: string,
    status: 'idle' | 'active' | 'waiting' | 'permission',
  ): void {
    const agentId = this.sessionToAgentId.get(sessionId);
    if (agentId === undefined) return;
    this.emit(agentId, { type: 'agent.statusChanged', id: agentId, status });
  }

  private emit(agentId: number, event: AgentEvent): void {
    this.sink.emitTo(agentId, event);
  }

  private pushTool(agentId: number, toolId: string): void {
    let stack = this.toolStack.get(agentId);
    if (!stack) {
      stack = [];
      this.toolStack.set(agentId, stack);
    }
    stack.push(toolId);
  }

  private popTool(agentId: number): string | undefined {
    const stack = this.toolStack.get(agentId);
    if (!stack || stack.length === 0) return undefined;
    return stack.pop();
  }
}

/**
 * Cheap one-liner like the extension's `formatToolStatus` so subscribers see
 * a usable string immediately. Kept inline (rather than imported from the CJS
 * claude provider) so the bridge stays ESM-only and dependency-light.
 */
function shortStatus(toolName: string, input: Record<string, unknown>): string {
  const base = (v: unknown): string => (typeof v === 'string' ? path.basename(v) : '');
  switch (toolName) {
    case 'Read':
      return `Reading ${base(input.file_path)}`;
    case 'Edit':
      return `Editing ${base(input.file_path)}`;
    case 'Write':
      return `Writing ${base(input.file_path)}`;
    case 'Bash': {
      const cmd = typeof input.command === 'string' ? input.command : '';
      return `Running: ${cmd.length > 30 ? cmd.slice(0, 30) + '…' : cmd}`;
    }
    case 'Glob':
      return 'Searching files';
    case 'Grep':
      return 'Searching code';
    case 'WebFetch':
      return 'Fetching web content';
    case 'WebSearch':
      return 'Searching the web';
    case 'Task':
    case 'Agent':
      return typeof input.description === 'string'
        ? `Subtask: ${input.description}`
        : 'Running subtask';
    default:
      return `Using ${toolName}`;
  }
}
