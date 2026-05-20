import { describe, expect, it, vi } from 'vitest';

import { RecordingSink } from '../../../src/messageSender.js';
import { createNullLogger } from '../../src/logging/logger.js';
import type { DispatchContext } from '../../src/rpc/dispatch.js';
import { handleAgentExit } from '../../src/rpc/methods/agents.js';

/**
 * WHO: agent.spawn live PTY exit handler (handleAgentExit).
 * WHAT: a fresh spawn that dies on a missing claude binary (exit 127) must
 *       emit agent.spawnFailed, not just a generic agent.exited.
 * WHY: previously only reviveAgentsOnBoot toasted spawnFailed; live spawns
 *      silently looked like normal exits, so the UI never warned the user.
 */
function makeCtx() {
  const sink = new RecordingSink();
  const removeAgent = vi.fn();
  const stopPoll = vi.fn();
  const ctx = {
    sink,
    liveAgents: { remove: vi.fn() },
    jsonlPoller: { stop: stopPoll },
    agents: { remove: removeAgent },
    logger: createNullLogger(),
  } as unknown as DispatchContext;
  return { ctx, sink, removeAgent, stopPoll };
}

describe('handleAgentExit', () => {
  const agent = { agentId: 5, cwd: '/work', sessionId: 'sess-5' };

  it('exit 127 emits agent.exited(claude_missing) AND agent.spawnFailed', () => {
    const { ctx, sink, stopPoll } = makeCtx();
    handleAgentExit(ctx, agent, 127, undefined);

    const exited = sink.byType('agent.exited');
    expect(exited).toHaveLength(1);
    expect(exited[0].reason).toBe('claude_missing');

    const failed = sink.byType('agent.spawnFailed');
    expect(failed).toHaveLength(1);
    expect(failed[0]).toMatchObject({ id: 5, sessionId: 'sess-5', reason: 'claude_missing' });

    expect(stopPoll).toHaveBeenCalledWith(5);
  });

  it('clean exit 0 emits exited(user), removes from persistence, no spawnFailed', () => {
    const { ctx, sink, removeAgent } = makeCtx();
    handleAgentExit(ctx, agent, 0, undefined);

    expect(sink.byType('agent.exited')[0].reason).toBe('user');
    expect(sink.byType('agent.spawnFailed')).toHaveLength(0);
    expect(removeAgent).toHaveBeenCalledWith('/work', 5);
  });

  it('crash (exit 1) emits exited(crash), keeps persistence, no spawnFailed', () => {
    const { ctx, sink, removeAgent } = makeCtx();
    handleAgentExit(ctx, agent, 1, undefined);

    expect(sink.byType('agent.exited')[0].reason).toBe('crash');
    expect(sink.byType('agent.spawnFailed')).toHaveLength(0);
    expect(removeAgent).not.toHaveBeenCalled();
  });
});
