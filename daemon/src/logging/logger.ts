import * as fs from 'fs';
import * as path from 'path';

import { LOG_LEVELS, type LogLevel } from '../config/persistence.js';

/**
 * NDJSON file logger for the daemon. One file per UTC day, rolled when the
 * date changes (lazily on the next `write()` — no timers). Arch §"Logging
 * format/path" (MIN-5) pins the on-disk shape:
 *
 *   {"ts":"2026-05-19T12:34:56.789Z","level":"info","module":"agents","agentId":7,"msg":"..."}
 *
 * Retention sweep (gz @ 7d, delete @ 30d) lives in `retention.ts` and is
 * scheduled from `server.ts` at boot + once/day.
 */

const LEVEL_RANK: Record<LogLevel, number> = {
  trace: 0,
  debug: 1,
  info: 2,
  warn: 3,
  error: 4,
};

export interface LogFields {
  module: string;
  /** Optional agent id, included as `agentId` in the NDJSON record. */
  agentId?: number;
  /** Any additional structured fields. Reserved names (`ts`, `level`, `msg`) are overwritten. */
  [key: string]: unknown;
}

export interface Logger {
  trace(fields: LogFields, msg: string): void;
  debug(fields: LogFields, msg: string): void;
  info(fields: LogFields, msg: string): void;
  warn(fields: LogFields, msg: string): void;
  error(fields: LogFields, msg: string): void;
  /** Update the minimum level. Called when config.json's logLevel changes. */
  setLevel(level: LogLevel): void;
  /** Flush + close the current file. Called from shutdown. */
  close(): void;
}

export interface FileLoggerOptions {
  /** Directory to write `daemon-YYYY-MM-DD.log` into. Created if missing. */
  dir: string;
  /** Filename prefix before the date suffix. e.g. `"daemon"` → `daemon-2026-05-19.log`. */
  prefix: string;
  /** Initial minimum level. */
  level: LogLevel;
  /**
   * Mirror records to a process stream in addition to the file. Used by
   * `--foreground` so operators see output without `tail -f`. The mirror is
   * raw NDJSON — same as the file.
   */
  mirror?: NodeJS.WritableStream;
  /**
   * Override the clock for tests. Defaults to `Date.now`. Day rollover is
   * decided by the UTC date string returned from this clock.
   */
  now?: () => Date;
}

export function utcDateString(d: Date): string {
  const yyyy = d.getUTCFullYear().toString().padStart(4, '0');
  const mm = (d.getUTCMonth() + 1).toString().padStart(2, '0');
  const dd = d.getUTCDate().toString().padStart(2, '0');
  return `${yyyy}-${mm}-${dd}`;
}

class FileLogger implements Logger {
  private level: LogLevel;
  private readonly dir: string;
  private readonly prefix: string;
  private readonly mirror?: NodeJS.WritableStream;
  private readonly now: () => Date;
  private fd: number | null = null;
  private fdDate = '';
  private closed = false;

  constructor(opts: FileLoggerOptions) {
    this.dir = opts.dir;
    this.prefix = opts.prefix;
    this.level = opts.level;
    this.mirror = opts.mirror;
    this.now = opts.now ?? (() => new Date());
    fs.mkdirSync(this.dir, { recursive: true });
  }

  trace(fields: LogFields, msg: string): void {
    this.write('trace', fields, msg);
  }
  debug(fields: LogFields, msg: string): void {
    this.write('debug', fields, msg);
  }
  info(fields: LogFields, msg: string): void {
    this.write('info', fields, msg);
  }
  warn(fields: LogFields, msg: string): void {
    this.write('warn', fields, msg);
  }
  error(fields: LogFields, msg: string): void {
    this.write('error', fields, msg);
  }

  setLevel(level: LogLevel): void {
    if (!LOG_LEVELS.includes(level)) return;
    this.level = level;
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.closeFd();
  }

  private closeFd(): void {
    if (this.fd !== null) {
      try {
        fs.closeSync(this.fd);
      } catch {
        // best effort
      }
      this.fd = null;
      this.fdDate = '';
    }
  }

  private write(level: LogLevel, fields: LogFields, msg: string): void {
    if (this.closed) return;
    if (LEVEL_RANK[level] < LEVEL_RANK[this.level]) return;

    const now = this.now();
    const record: Record<string, unknown> = {
      ...fields,
      ts: now.toISOString(),
      level,
      msg,
    };
    // Re-pin the canonical key order so `ts` / `level` come first regardless
    // of what the caller put on `fields`.
    const ordered: Record<string, unknown> = {
      ts: record.ts,
      level: record.level,
      module: record.module,
    };
    if (record.agentId !== undefined) ordered.agentId = record.agentId;
    for (const k of Object.keys(record)) {
      if (k in ordered || k === 'msg') continue;
      ordered[k] = record[k];
    }
    ordered.msg = record.msg;

    const line = JSON.stringify(ordered) + '\n';
    const fd = this.ensureFd(now);
    if (fd !== null) {
      try {
        fs.writeSync(fd, line);
      } catch {
        // Filesystem may have been yanked out from under us (rm -rf, full disk).
        // Close so the next write retries with a fresh open.
        this.closeFd();
      }
    }
    if (this.mirror) {
      try {
        this.mirror.write(line);
      } catch {
        // best effort
      }
    }
  }

  private ensureFd(now: Date): number | null {
    const date = utcDateString(now);
    if (this.fd !== null && this.fdDate === date) return this.fd;
    this.closeFd();

    const file = path.join(this.dir, `${this.prefix}-${date}.log`);
    try {
      // 0o600 so log lines (which may contain tokens or session ids) are
      // user-only on shared hosts. `a` is append-only; concurrent writes
      // from multiple FDs append atomically up to the OS pipe-buf size.
      this.fd = fs.openSync(file, 'a', 0o600);
      this.fdDate = date;
      return this.fd;
    } catch {
      return null;
    }
  }
}

export function createFileLogger(opts: FileLoggerOptions): Logger {
  return new FileLogger(opts);
}

/** Logger that drops every record. Used by tests and by code paths that boot before the real logger is wired. */
export function createNullLogger(): Logger {
  return {
    trace() {},
    debug() {},
    info() {},
    warn() {},
    error() {},
    setLevel() {},
    close() {},
  };
}
