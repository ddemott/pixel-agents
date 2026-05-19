export interface TerminalLike {
  readonly name: string;
  readonly exitStatus: { readonly code: number | undefined } | undefined;
  show(preserveFocus?: boolean): void;
  sendText(text: string, addNewLine?: boolean): void;
  dispose(): void;
}

export interface TerminalCreateOptions {
  name: string;
  cwd?: string;
}

export interface TerminalRegistry {
  getActive(): TerminalLike | undefined;
  list(): readonly TerminalLike[];
  create(opts: TerminalCreateOptions): TerminalLike;
}

export class NullTerminalRegistry implements TerminalRegistry {
  getActive(): TerminalLike | undefined {
    return undefined;
  }
  list(): readonly TerminalLike[] {
    return [];
  }
  create(_opts: TerminalCreateOptions): TerminalLike {
    throw new Error('NullTerminalRegistry cannot create terminals — register a real impl first');
  }
}

let active: TerminalRegistry = new NullTerminalRegistry();

export function setTerminalRegistry(registry: TerminalRegistry): void {
  active = registry;
}

export function getTerminalRegistry(): TerminalRegistry {
  return active;
}
