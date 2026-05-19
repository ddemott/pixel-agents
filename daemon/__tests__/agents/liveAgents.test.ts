import { describe, expect, it } from 'vitest';

import { LiveAgents } from '../../src/agents/liveAgents.js';

describe('LiveAgents', () => {
  it('allocates monotonically increasing ids', () => {
    const r = new LiveAgents();
    expect(r.allocateId()).toBe(1);
    expect(r.allocateId()).toBe(2);
    expect(r.allocateId()).toBe(3);
  });

  it('reserveId bumps the counter past revived ids', () => {
    const r = new LiveAgents();
    r.reserveId(10);
    expect(r.allocateId()).toBe(11);
  });

  it('add/get/remove round-trip', () => {
    const r = new LiveAgents();
    const id = r.allocateId();
    const fakePty = { kill() {} } as never;
    r.add({ id, sessionId: 's', cwd: '/', startedAt: 0, pty: fakePty });
    expect(r.size()).toBe(1);
    expect(r.get(id)?.sessionId).toBe('s');
    expect(r.bySession('s')?.id).toBe(id);
    expect(r.remove(id)?.sessionId).toBe('s');
    expect(r.size()).toBe(0);
    expect(r.get(id)).toBeUndefined();
    expect(r.bySession('s')).toBeUndefined();
  });

  it('rejects duplicate id and duplicate sessionId', () => {
    const r = new LiveAgents();
    const fakePty = { kill() {} } as never;
    r.add({ id: 1, sessionId: 'a', cwd: '/', startedAt: 0, pty: fakePty });
    expect(() => r.add({ id: 1, sessionId: 'b', cwd: '/', startedAt: 0, pty: fakePty })).toThrow();
    expect(() => r.add({ id: 2, sessionId: 'a', cwd: '/', startedAt: 0, pty: fakePty })).toThrow();
  });
});
