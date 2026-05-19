import { PERMISSION_TIMER_DELAY_MS } from '../server/src/constants.js';
import type { AgentEventSink } from './messageSender.js';
import type { AgentState } from './types.js';

export function clearAgentActivity(
  agent: AgentState | undefined,
  agentId: number,
  permissionTimers: Map<number, ReturnType<typeof setTimeout>>,
  sink: AgentEventSink,
): void {
  if (!agent) return;

  // Preserve background agent tools — only clear foreground state
  if (agent.backgroundAgentToolIds.size > 0) {
    for (const toolId of agent.activeToolIds) {
      if (agent.backgroundAgentToolIds.has(toolId)) continue;
      agent.activeToolIds.delete(toolId);
      agent.activeToolStatuses.delete(toolId);
      const toolName = agent.activeToolNames.get(toolId);
      agent.activeToolNames.delete(toolId);
      if (toolName === 'Task' || toolName === 'Agent') {
        agent.activeSubagentToolIds.delete(toolId);
        agent.activeSubagentToolNames.delete(toolId);
      }
    }
  } else {
    agent.activeToolIds.clear();
    agent.activeToolStatuses.clear();
    agent.activeToolNames.clear();
    agent.activeSubagentToolIds.clear();
    agent.activeSubagentToolNames.clear();
  }

  agent.isWaiting = false;
  agent.permissionSent = false;
  cancelPermissionTimer(agentId, permissionTimers);
  sink.post({ type: 'agentToolsClear', id: agentId });
  // Re-send background agent tools so the UI re-creates their sub-agents
  for (const toolId of agent.backgroundAgentToolIds) {
    const status = agent.activeToolStatuses.get(toolId);
    if (status) {
      sink.post({
        type: 'agentToolStart',
        id: agentId,
        toolId,
        status,
      });
    }
  }
  sink.post({ type: 'agentStatus', id: agentId, status: 'active' });
}

export function cancelWaitingTimer(
  agentId: number,
  waitingTimers: Map<number, ReturnType<typeof setTimeout>>,
): void {
  const timer = waitingTimers.get(agentId);
  if (timer) {
    clearTimeout(timer);
    waitingTimers.delete(agentId);
  }
}

export function startWaitingTimer(
  agentId: number,
  delayMs: number,
  agents: Map<number, AgentState>,
  waitingTimers: Map<number, ReturnType<typeof setTimeout>>,
  sink: AgentEventSink,
): void {
  cancelWaitingTimer(agentId, waitingTimers);
  const timer = setTimeout(() => {
    waitingTimers.delete(agentId);
    const agent = agents.get(agentId);
    if (agent) {
      agent.isWaiting = true;
    }
    sink.post({
      type: 'agentStatus',
      id: agentId,
      status: 'waiting',
    });
  }, delayMs);
  waitingTimers.set(agentId, timer);
}

export function cancelPermissionTimer(
  agentId: number,
  permissionTimers: Map<number, ReturnType<typeof setTimeout>>,
): void {
  const timer = permissionTimers.get(agentId);
  if (timer) {
    clearTimeout(timer);
    permissionTimers.delete(agentId);
  }
}

export function startPermissionTimer(
  agentId: number,
  agents: Map<number, AgentState>,
  permissionTimers: Map<number, ReturnType<typeof setTimeout>>,
  permissionExemptTools: Set<string>,
  sink: AgentEventSink,
): void {
  cancelPermissionTimer(agentId, permissionTimers);
  const timer = setTimeout(() => {
    permissionTimers.delete(agentId);
    const agent = agents.get(agentId);
    if (!agent) return;

    // Only flag if there are still active non-exempt tools (parent or sub-agent)
    let hasNonExempt = false;
    for (const toolId of agent.activeToolIds) {
      const toolName = agent.activeToolNames.get(toolId);
      if (!permissionExemptTools.has(toolName || '')) {
        hasNonExempt = true;
        break;
      }
    }

    // Check sub-agent tools for non-exempt tools
    const stuckSubagentParentToolIds: string[] = [];
    for (const [parentToolId, subToolNames] of agent.activeSubagentToolNames) {
      for (const [, toolName] of subToolNames) {
        if (!permissionExemptTools.has(toolName)) {
          stuckSubagentParentToolIds.push(parentToolId);
          hasNonExempt = true;
          break;
        }
      }
    }

    if (hasNonExempt) {
      agent.permissionSent = true;
      console.log(`[Pixel Agents] Timer: Agent ${agentId} - possible permission wait detected`);
      sink.post({
        type: 'agentToolPermission',
        id: agentId,
      });
      // Also notify stuck sub-agents
      for (const parentToolId of stuckSubagentParentToolIds) {
        sink.post({
          type: 'subagentToolPermission',
          id: agentId,
          parentToolId,
        });
      }
    }
  }, PERMISSION_TIMER_DELAY_MS);
  permissionTimers.set(agentId, timer);
}
