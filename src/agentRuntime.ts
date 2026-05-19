export interface WorkspaceFolder {
  readonly fsPath: string;
  readonly name: string;
}

export interface AgentRuntime {
  getWorkspaceFolders(): readonly WorkspaceFolder[];
}

export interface AgentStateStore {
  get<T>(key: string): T | undefined;
  set(key: string, value: unknown): Thenable<void> | Promise<void> | void;
}

export class NullAgentRuntime implements AgentRuntime {
  getWorkspaceFolders(): readonly WorkspaceFolder[] {
    return [];
  }
}

export class InMemoryAgentStateStore implements AgentStateStore {
  private readonly store = new Map<string, unknown>();
  get<T>(key: string): T | undefined {
    return this.store.get(key) as T | undefined;
  }
  set(key: string, value: unknown): void {
    this.store.set(key, value);
  }
}

let activeRuntime: AgentRuntime = new NullAgentRuntime();

export function setAgentRuntime(runtime: AgentRuntime): void {
  activeRuntime = runtime;
}

export function getAgentRuntime(): AgentRuntime {
  return activeRuntime;
}
