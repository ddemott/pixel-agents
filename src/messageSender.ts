import type * as vscode from 'vscode';

export interface AgentEvent {
  type: string;
  [key: string]: unknown;
}

export interface AgentEventSink {
  post(event: AgentEvent): void;
}

export class WebviewSink implements AgentEventSink {
  constructor(private readonly webview: vscode.Webview) {}

  post(event: AgentEvent): void {
    void this.webview.postMessage(event);
  }
}

export class NullSink implements AgentEventSink {
  post(_event: AgentEvent): void {}
}

export class RecordingSink implements AgentEventSink {
  readonly events: AgentEvent[] = [];

  post(event: AgentEvent): void {
    this.events.push(event);
  }

  clear(): void {
    this.events.length = 0;
  }

  byType(type: string): AgentEvent[] {
    return this.events.filter((e) => e.type === type);
  }
}

export function sinkFromWebview(webview: vscode.Webview | undefined): AgentEventSink {
  return webview ? new WebviewSink(webview) : new NullSink();
}
