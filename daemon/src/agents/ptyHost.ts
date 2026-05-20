import * as pty from 'node-pty';

import type { Logger } from '../logging/logger.js';

/**
 * Thin wrapper around a `node-pty` handle that owns a single `claude` (or
 * other) child process. The daemon attaches:
 *   - an `onData` callback that ships raw bytes to subscribed clients via
 *     the binary multiplex (tag 0x01); the data callback is invoked
 *     synchronously by node-pty on every chunk.
 *   - an `onExit` callback that fires once with the child's exit code.
 *
 * The host is intentionally thin so test code can pass a mocked `spawn`
 * function instead of pulling in real `node-pty` for unit tests.
 */

export interface PtyHostOptions {
  agentId: number;
  command: string;
  args: string[];
  cwd: string;
  env?: NodeJS.ProcessEnv;
  cols?: number;
  rows?: number;
  logger: Logger;
  /** Test seam: override the underlying spawn function. Defaults to `node-pty`. */
  spawn?: SpawnFn;
}

export interface PtyHostCallbacks {
  /** Raw bytes from the child. Daemon forwards over `BroadcastSink.broadcastPty`. */
  onData: (bytes: Buffer) => void;
  /** Fired once when the child exits. The host no longer accepts writes after this. */
  onExit: (exitCode: number, signal?: number) => void;
}

/** Subset of the `node-pty` `IPty` surface we depend on. */
export interface PtyHandle {
  readonly pid: number;
  onData(cb: (data: string | Buffer) => void): { dispose(): void };
  onExit(cb: (e: { exitCode: number; signal?: number }) => void): { dispose(): void };
  write(data: string): void;
  resize(cols: number, rows: number): void;
  kill(signal?: string): void;
  /** node-pty flow control — pause/resume the child's output stream. */
  pause?(): void;
  resume?(): void;
}

export type SpawnFn = (
  command: string,
  args: string[],
  options: {
    cwd?: string;
    env?: NodeJS.ProcessEnv;
    cols?: number;
    rows?: number;
    encoding?: BufferEncoding | null;
  },
) => PtyHandle;

const DEFAULT_COLS = 120;
const DEFAULT_ROWS = 40;

const defaultSpawn: SpawnFn = (cmd, args, opts) =>
  pty.spawn(cmd, args, {
    cwd: opts.cwd,
    env: opts.env as { [key: string]: string },
    cols: opts.cols ?? DEFAULT_COLS,
    rows: opts.rows ?? DEFAULT_ROWS,
    encoding: null as unknown as BufferEncoding,
    // `claude` checks isTTY; node-pty allocates a PTY so this is satisfied.
  }) as unknown as PtyHandle;

export class PtyHost {
  readonly agentId: number;
  readonly pid: number;
  private readonly handle: PtyHandle;
  private readonly logger: Logger;
  private dataSub: { dispose(): void } | null = null;
  private exitSub: { dispose(): void } | null = null;
  private exited = false;

  constructor(opts: PtyHostOptions, callbacks: PtyHostCallbacks) {
    this.agentId = opts.agentId;
    this.logger = opts.logger;

    const spawnFn = opts.spawn ?? defaultSpawn;
    this.handle = spawnFn(opts.command, opts.args, {
      cwd: opts.cwd,
      env: opts.env,
      cols: opts.cols ?? DEFAULT_COLS,
      rows: opts.rows ?? DEFAULT_ROWS,
      encoding: null,
    });
    this.pid = this.handle.pid;

    this.dataSub = this.handle.onData((chunk) => {
      const buf = typeof chunk === 'string' ? Buffer.from(chunk, 'utf-8') : chunk;
      try {
        callbacks.onData(buf);
      } catch (e) {
        this.logger.error(
          {
            module: 'ptyHost',
            agentId: opts.agentId,
            error: e instanceof Error ? e.message : String(e),
          },
          'onData callback threw',
        );
      }
    });

    this.exitSub = this.handle.onExit(({ exitCode, signal }) => {
      this.exited = true;
      this.dataSub?.dispose();
      this.exitSub?.dispose();
      this.dataSub = null;
      this.exitSub = null;
      callbacks.onExit(exitCode, signal);
    });

    this.logger.info(
      {
        module: 'ptyHost',
        agentId: opts.agentId,
        pid: this.pid,
        command: opts.command,
      },
      'pty spawned',
    );
  }

  /** Write bytes to the child's stdin. No-op if the PTY has already exited. */
  write(data: Buffer | string): void {
    if (this.exited) return;
    const s = typeof data === 'string' ? data : data.toString('binary');
    try {
      this.handle.write(s);
    } catch (e) {
      this.logger.warn(
        {
          module: 'ptyHost',
          agentId: this.agentId,
          error: e instanceof Error ? e.message : String(e),
        },
        'write threw',
      );
    }
  }

  /** Resize the PTY window. No-op if the PTY has already exited. */
  resize(cols: number, rows: number): void {
    if (this.exited) return;
    try {
      this.handle.resize(cols, rows);
    } catch (e) {
      this.logger.warn(
        {
          module: 'ptyHost',
          agentId: this.agentId,
          error: e instanceof Error ? e.message : String(e),
        },
        'resize threw',
      );
    }
  }

  /**
   * Send a signal to the child. Default `SIGTERM`; callers escalate to
   * `SIGKILL` after a grace period if `onExit` doesn't fire.
   */
  kill(signal: string = 'SIGTERM'): void {
    if (this.exited) return;
    try {
      this.handle.kill(signal);
    } catch (e) {
      this.logger.warn(
        {
          module: 'ptyHost',
          agentId: this.agentId,
          error: e instanceof Error ? e.message : String(e),
        },
        'kill threw',
      );
    }
  }

  /**
   * Pause the child's output via node-pty flow control. No-op after exit, or if
   * the underlying handle predates pause/resume support. Dormant capability: no
   * caller gates on backpressure today (see `BackpressureCallbacks` rationale —
   * the per-subscriber ring is the OOM ceiling), but the hook exists for future
   * per-agent flow control without re-plumbing.
   */
  pause(): void {
    if (this.exited) return;
    try {
      this.handle.pause?.();
    } catch (e) {
      this.logger.warn(
        {
          module: 'ptyHost',
          agentId: this.agentId,
          error: e instanceof Error ? e.message : String(e),
        },
        'pause threw',
      );
    }
  }

  /** Resume a paused child's output. No-op after exit. Companion to `pause`. */
  resume(): void {
    if (this.exited) return;
    try {
      this.handle.resume?.();
    } catch (e) {
      this.logger.warn(
        {
          module: 'ptyHost',
          agentId: this.agentId,
          error: e instanceof Error ? e.message : String(e),
        },
        'resume threw',
      );
    }
  }

  isAlive(): boolean {
    return !this.exited;
  }
}
