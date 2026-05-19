import type { PtyHost } from './ptyHost.js';

/**
 * In-memory live-agent registry. Owns the running PTY processes for the
 * duration of a daemon run.
 *
 * Separate from `AgentsRegistry` (which owns the on-disk `agents.json` per-cwd
 * index used for revival across restarts): one is "what's running right now",
 * the other is "what we promised to restart on next boot". `agent.spawn`
 * inserts into both; `agent.close` removes from both; an unexpected PTY exit
 * removes from this registry but leaves the persisted record alone so
 * `--resume` (Day 15-16) can attempt revival.
 */

export interface LiveAgent {
  id: number;
  sessionId: string;
  cwd: string;
  startedAt: number;
  pty: PtyHost;
}

export class LiveAgents {
  private readonly byId = new Map<number, LiveAgent>();
  private readonly bySessionId = new Map<string, number>();
  private nextId = 1;

  /** Allocate the next agent id. Caller is responsible for inserting via `add`. */
  allocateId(): number {
    return this.nextId++;
  }

  /**
   * Reserve a specific id (used when reviving from `agents.json` — see Day
   * 15-16). Bumps the counter so future allocations don't collide.
   */
  reserveId(id: number): void {
    if (id >= this.nextId) this.nextId = id + 1;
  }

  add(agent: LiveAgent): void {
    if (this.byId.has(agent.id)) {
      throw new Error(`live agent ${agent.id} already registered`);
    }
    if (this.bySessionId.has(agent.sessionId)) {
      throw new Error(`session ${agent.sessionId} already mapped`);
    }
    this.byId.set(agent.id, agent);
    this.bySessionId.set(agent.sessionId, agent.id);
  }

  get(id: number): LiveAgent | undefined {
    return this.byId.get(id);
  }

  bySession(sessionId: string): LiveAgent | undefined {
    const id = this.bySessionId.get(sessionId);
    return id === undefined ? undefined : this.byId.get(id);
  }

  list(): LiveAgent[] {
    return Array.from(this.byId.values());
  }

  remove(id: number): LiveAgent | undefined {
    const a = this.byId.get(id);
    if (!a) return undefined;
    this.byId.delete(id);
    this.bySessionId.delete(a.sessionId);
    return a;
  }

  size(): number {
    return this.byId.size;
  }
}
