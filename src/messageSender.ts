import type * as vscode from 'vscode';

export interface AgentEvent {
  type: string;
  [key: string]: unknown;
}

export interface AgentEventSink {
  /** Fan out to every subscriber (subject to per-conn topic filters). */
  post(event: AgentEvent): void;
  /**
   * Targeted variant scoped to a single `agentId`. Implementations that don't
   * model per-agent subscriptions (webview, recorders) treat this as `post`.
   * The daemon's `BroadcastSink` uses it to deliver only to connections that
   * opted into `agent:<id>` (or `agent:*`, or unfiltered).
   */
  emitTo(agentId: number, event: AgentEvent): void;
}

export class WebviewSink implements AgentEventSink {
  constructor(private readonly webview: vscode.Webview) {}

  post(event: AgentEvent): void {
    void this.webview.postMessage(event);
  }

  emitTo(_agentId: number, event: AgentEvent): void {
    this.post(event);
  }
}

export class NullSink implements AgentEventSink {
  post(_event: AgentEvent): void {}

  emitTo(_agentId: number, _event: AgentEvent): void {}
}

export class RecordingSink implements AgentEventSink {
  readonly events: AgentEvent[] = [];
  readonly targeted: Array<{ agentId: number; event: AgentEvent }> = [];

  post(event: AgentEvent): void {
    this.events.push(event);
  }

  emitTo(agentId: number, event: AgentEvent): void {
    this.targeted.push({ agentId, event });
    this.events.push(event);
  }

  clear(): void {
    this.events.length = 0;
    this.targeted.length = 0;
  }

  byType(type: string): AgentEvent[] {
    return this.events.filter((e) => e.type === type);
  }
}

export function sinkFromWebview(webview: vscode.Webview | undefined): AgentEventSink {
  return webview ? new WebviewSink(webview) : new NullSink();
}
