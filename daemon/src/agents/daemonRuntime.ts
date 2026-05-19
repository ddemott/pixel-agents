import type { AgentRuntime, WorkspaceFolder } from '../../../src/agentRuntime.js';

/**
 * Daemon-side AgentRuntime. The daemon is workspace-agnostic — it serves
 * agents from any cwd. For Day 5 we report a single folder corresponding to
 * the daemon's own cwd at boot. Multi-folder support arrives once the RPC
 * command catalog (Day 7-8) introduces `agent.spawn { cwd }`.
 */
export class DaemonRuntime implements AgentRuntime {
  constructor(private readonly bootCwd: string) {}

  getWorkspaceFolders(): readonly WorkspaceFolder[] {
    return [{ fsPath: this.bootCwd, name: pathBasename(this.bootCwd) }];
  }
}

function pathBasename(p: string): string {
  const trimmed = p.replace(/[\\/]+$/, '');
  const lastSep = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return lastSep >= 0 ? trimmed.slice(lastSep + 1) : trimmed;
}
