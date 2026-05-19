import * as vscode from 'vscode';

import { type AgentRuntime, setAgentRuntime, type WorkspaceFolder } from './agentRuntime.js';
import { COMMAND_EXPORT_DEFAULT_LAYOUT, COMMAND_SHOW_PANEL, VIEW_ID } from './constants.js';
import { PixelAgentsViewProvider } from './PixelAgentsViewProvider.js';
import {
  setTerminalRegistry,
  type TerminalCreateOptions,
  type TerminalLike,
  type TerminalRegistry,
} from './terminalRegistry.js';

let providerInstance: PixelAgentsViewProvider | undefined;

class VsCodeTerminalRegistry implements TerminalRegistry {
  getActive(): TerminalLike | undefined {
    return vscode.window.activeTerminal;
  }
  list(): readonly TerminalLike[] {
    return vscode.window.terminals;
  }
  create(opts: TerminalCreateOptions): TerminalLike {
    return vscode.window.createTerminal({ name: opts.name, cwd: opts.cwd });
  }
}

class VsCodeAgentRuntime implements AgentRuntime {
  getWorkspaceFolders(): readonly WorkspaceFolder[] {
    return (vscode.workspace.workspaceFolders ?? []).map((f) => ({
      fsPath: f.uri.fsPath,
      name: f.name,
    }));
  }
}

export function activate(context: vscode.ExtensionContext) {
  console.log(`[Pixel Agents] PIXEL_AGENTS_DEBUG=${process.env.PIXEL_AGENTS_DEBUG ?? 'not set'}`);
  setTerminalRegistry(new VsCodeTerminalRegistry());
  setAgentRuntime(new VsCodeAgentRuntime());
  const provider = new PixelAgentsViewProvider(context);
  providerInstance = provider;

  context.subscriptions.push(vscode.window.registerWebviewViewProvider(VIEW_ID, provider));

  context.subscriptions.push(
    vscode.commands.registerCommand(COMMAND_SHOW_PANEL, () => {
      vscode.commands.executeCommand(`${VIEW_ID}.focus`);
    }),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand(COMMAND_EXPORT_DEFAULT_LAYOUT, () => {
      provider.exportDefaultLayout();
    }),
  );
}

export function deactivate() {
  providerInstance?.dispose();
}
