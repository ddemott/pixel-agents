import { AGENTS_JSON_PATH } from '../paths.js';
import { readTagged, type WriterTag, writeTagged } from '../persistence/writerTag.js';

/**
 * agents.json — per-cwd index of agents that should be revived on daemon
 * restart. Schema per arch §16:
 *
 *   { version: 1, agents: { "<cwd>": PersistedAgent[] }, _writer: {...} }
 *
 * Day 6 lands the schema + read/write path. Day 15-16 wires `claude --resume`
 * revival on boot. Day 13-14 keeps it in sync with live spawns/closes.
 */

export interface PersistedAgent {
  id: number;
  sessionId: string;
  palette: number;
  hueShift: number;
  seatId?: string;
  isExternal?: boolean;
  folderName?: string;
  /** Recorded so we can distinguish "old" entries from a previous daemon boot. */
  lastSeenAt: number;
}

interface AgentsRegistryShape extends Record<string, unknown> {
  version: 1;
  agents: Record<string, PersistedAgent[]>;
}

const DEFAULT_REGISTRY: AgentsRegistryShape = { version: 1, agents: {} };

export class AgentsRegistry {
  private data: AgentsRegistryShape;

  constructor(private readonly ours: WriterTag) {
    this.data = loadOrDefault();
  }

  /** Snapshot of agents for the given cwd. */
  forCwd(cwd: string): PersistedAgent[] {
    return [...(this.data.agents[cwd] ?? [])];
  }

  /** Replace the agent list for a cwd atomically. */
  setCwd(cwd: string, agents: PersistedAgent[]): void {
    this.data.agents[cwd] = agents;
    this.flush();
  }

  /** Upsert a single agent by id within a cwd. */
  upsert(cwd: string, agent: PersistedAgent): void {
    const list = this.data.agents[cwd] ?? [];
    const idx = list.findIndex((a) => a.id === agent.id);
    if (idx === -1) list.push(agent);
    else list[idx] = agent;
    this.data.agents[cwd] = list;
    this.flush();
  }

  /** Remove an agent by id from a cwd. Returns true when an entry was removed. */
  remove(cwd: string, id: number): boolean {
    const list = this.data.agents[cwd];
    if (!list) return false;
    const filtered = list.filter((a) => a.id !== id);
    if (filtered.length === list.length) return false;
    if (filtered.length === 0) delete this.data.agents[cwd];
    else this.data.agents[cwd] = filtered;
    this.flush();
    return true;
  }

  /** All cwds with at least one persisted agent. */
  cwds(): string[] {
    return Object.keys(this.data.agents);
  }

  private flush(): void {
    writeTagged(AGENTS_JSON_PATH, this.data as Record<string, unknown>, this.ours);
  }
}

function loadOrDefault(): AgentsRegistryShape {
  const result = readTagged<AgentsRegistryShape>(AGENTS_JSON_PATH);
  if (!result) return { ...DEFAULT_REGISTRY, agents: {} };
  const d = result.data;
  if (d.version !== 1 || typeof d.agents !== 'object' || d.agents === null) {
    // Malformed / unknown version: do not crash boot. Day 7+ can introduce
    // migrations as the schema evolves.
    return { ...DEFAULT_REGISTRY, agents: {} };
  }
  return { version: 1, agents: d.agents };
}
