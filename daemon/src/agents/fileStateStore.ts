import * as fs from 'fs';
import * as path from 'path';

import type { AgentStateStore } from '../../../src/agentRuntime.js';

/**
 * AgentStateStore backed by a single JSON file. Atomic write via tmp + rename.
 * Used by the daemon for the `agents.json` registry. The full per-cwd schema
 * arrives in Day 6; for now this is a flat key/value bag that mirrors what
 * `vscode.ExtensionContext.workspaceState` exposes.
 *
 * In-memory cache: every `set` writes through to disk; every `get` reads from
 * the cache. The file is loaded once at construction.
 */
export class FileStateStore implements AgentStateStore {
  private cache: Record<string, unknown>;

  constructor(private readonly filePath: string) {
    this.cache = loadOrEmpty(filePath);
  }

  get<T>(key: string): T | undefined {
    return this.cache[key] as T | undefined;
  }

  set(key: string, value: unknown): void {
    if (value === undefined) {
      delete this.cache[key];
    } else {
      this.cache[key] = value;
    }
    writeAtomic(this.filePath, this.cache);
  }
}

function loadOrEmpty(filePath: string): Record<string, unknown> {
  try {
    if (!fs.existsSync(filePath)) return {};
    const raw = fs.readFileSync(filePath, 'utf-8');
    const parsed = JSON.parse(raw) as unknown;
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
    return {};
  } catch {
    // Malformed file: don't crash boot. Day 6 will add migration + backup.
    return {};
  }
}

function writeAtomic(filePath: string, data: Record<string, unknown>): void {
  const dir = path.dirname(filePath);
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  const tmp = filePath + '.tmp';
  fs.writeFileSync(tmp, JSON.stringify(data, null, 2), { mode: 0o600 });
  fs.renameSync(tmp, filePath);
}
