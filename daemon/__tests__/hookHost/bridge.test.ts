import { describe, expect, it } from 'vitest';

import { RecordingSink } from '../../../src/messageSender.js';
import { DaemonHookBridge } from '../../src/hookHost/bridge.js';
import { createNullLogger } from '../../src/logging/logger.js';

/**
 * WHO: DaemonHookBridge SubagentStart/Stop handling.
 * WHAT: these claude hook events were dropped (default debug-log branch);
 *       now they emit agent.subagentStart / agent.subagentEnd to the parent.
 * WHY: clients need the parent character's Task subtask state.
 */
describe('DaemonHookBridge subagent events', () => {
  function setup() {
    const sink = new RecordingSink();
    const bridge = new DaemonHookBridge(sink, createNullLogger());
    bridge.registerSession('sess-1', 7); // parent agent id 7
    return { sink, bridge };
  }

  it('emits agent.subagentStart scoped to the parent agent', () => {
    const { sink, bridge } = setup();
    bridge.handleEvent('claude', {
      hook_event_name: 'SubagentStart',
      session_id: 'sess-1',
      tool_use_id: 'call_abc',
    });
    const evt = sink.byType('agent.subagentStart');
    expect(evt).toHaveLength(1);
    expect(evt[0].id).toBe(7);
    expect(evt[0].toolUseId).toBe('call_abc');
    // Targeted to the parent agent so per-agent subscribers receive it.
    expect(
      sink.targeted.some((t) => t.agentId === 7 && t.event.type === 'agent.subagentStart'),
    ).toBe(true);
  });

  it('emits agent.subagentEnd on SubagentStop', () => {
    const { sink, bridge } = setup();
    bridge.handleEvent('claude', { hook_event_name: 'SubagentStop', session_id: 'sess-1' });
    const evt = sink.byType('agent.subagentEnd');
    expect(evt).toHaveLength(1);
    expect(evt[0].id).toBe(7);
    expect(evt[0].toolUseId).toBeUndefined();
  });

  it('drops subagent events for unknown sessions', () => {
    const { sink, bridge } = setup();
    bridge.handleEvent('claude', { hook_event_name: 'SubagentStart', session_id: 'who?' });
    expect(sink.byType('agent.subagentStart')).toHaveLength(0);
  });
});
