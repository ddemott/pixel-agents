import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { createFileLogger, utcDateString } from '../../src/logging/logger.js';

let tmpDir: string;

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-logger-'));
});

afterEach(() => {
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

function readLog(file: string): Record<string, unknown>[] {
  if (!fs.existsSync(file)) return [];
  return fs
    .readFileSync(file, 'utf-8')
    .split('\n')
    .filter(Boolean)
    .map((line) => JSON.parse(line) as Record<string, unknown>);
}

function fileFor(dir: string, date: Date): string {
  return path.join(dir, `daemon-${utcDateString(date)}.log`);
}

describe('FileLogger', () => {
  it('writes NDJSON records with the canonical key order', () => {
    const fixedDate = new Date('2026-05-19T12:34:56.789Z');
    const logger = createFileLogger({
      dir: tmpDir,
      prefix: 'daemon',
      level: 'trace',
      now: () => fixedDate,
    });
    logger.info({ module: 'agents', agentId: 7, extra: 'k' }, 'hello');
    logger.close();

    const records = readLog(fileFor(tmpDir, fixedDate));
    expect(records).toHaveLength(1);
    const r = records[0];
    expect(r).toEqual({
      ts: '2026-05-19T12:34:56.789Z',
      level: 'info',
      module: 'agents',
      agentId: 7,
      extra: 'k',
      msg: 'hello',
    });
    // Pinned order: ts, level, module, agentId, ..., msg last.
    expect(Object.keys(r)).toEqual(['ts', 'level', 'module', 'agentId', 'extra', 'msg']);
  });

  it('filters records below the configured level', () => {
    const fixedDate = new Date('2026-05-19T00:00:00Z');
    const logger = createFileLogger({
      dir: tmpDir,
      prefix: 'daemon',
      level: 'warn',
      now: () => fixedDate,
    });
    logger.trace({ module: 'x' }, 'should drop');
    logger.debug({ module: 'x' }, 'should drop');
    logger.info({ module: 'x' }, 'should drop');
    logger.warn({ module: 'x' }, 'keep-warn');
    logger.error({ module: 'x' }, 'keep-error');
    logger.close();

    const records = readLog(fileFor(tmpDir, fixedDate));
    expect(records.map((r) => r.msg)).toEqual(['keep-warn', 'keep-error']);
    expect(records.map((r) => r.level)).toEqual(['warn', 'error']);
  });

  it('setLevel raises and lowers the floor at runtime', () => {
    const fixedDate = new Date('2026-05-19T00:00:00Z');
    const logger = createFileLogger({
      dir: tmpDir,
      prefix: 'daemon',
      level: 'info',
      now: () => fixedDate,
    });
    logger.debug({ module: 'x' }, 'drop1');
    logger.setLevel('debug');
    logger.debug({ module: 'x' }, 'keep1');
    logger.setLevel('error');
    logger.warn({ module: 'x' }, 'drop2');
    logger.error({ module: 'x' }, 'keep2');
    logger.close();

    const records = readLog(fileFor(tmpDir, fixedDate));
    expect(records.map((r) => r.msg)).toEqual(['keep1', 'keep2']);
  });

  it('rolls over to a new file when the UTC date changes', () => {
    let current = new Date('2026-05-19T23:59:50Z');
    const logger = createFileLogger({
      dir: tmpDir,
      prefix: 'daemon',
      level: 'trace',
      now: () => current,
    });
    logger.info({ module: 'x' }, 'day1');
    current = new Date('2026-05-20T00:00:01Z');
    logger.info({ module: 'x' }, 'day2');
    logger.close();

    const day1 = readLog(path.join(tmpDir, 'daemon-2026-05-19.log'));
    const day2 = readLog(path.join(tmpDir, 'daemon-2026-05-20.log'));
    expect(day1.map((r) => r.msg)).toEqual(['day1']);
    expect(day2.map((r) => r.msg)).toEqual(['day2']);
  });

  it('mirrors records to the optional stream', () => {
    const chunks: string[] = [];
    const mirror = {
      write(buf: string | Buffer): boolean {
        chunks.push(typeof buf === 'string' ? buf : buf.toString('utf-8'));
        return true;
      },
    } as unknown as NodeJS.WritableStream;

    const logger = createFileLogger({
      dir: tmpDir,
      prefix: 'daemon',
      level: 'info',
      mirror,
      now: () => new Date('2026-05-19T00:00:00Z'),
    });
    logger.info({ module: 'x' }, 'mirrored');
    logger.close();

    expect(chunks).toHaveLength(1);
    const parsed = JSON.parse(chunks[0]) as Record<string, unknown>;
    expect(parsed.msg).toBe('mirrored');
  });

  it('drops writes after close() without throwing', () => {
    const logger = createFileLogger({
      dir: tmpDir,
      prefix: 'daemon',
      level: 'trace',
      now: () => new Date('2026-05-19T00:00:00Z'),
    });
    logger.info({ module: 'x' }, 'before');
    logger.close();
    expect(() => logger.info({ module: 'x' }, 'after')).not.toThrow();
  });

  it('creates the log directory if it does not exist yet', () => {
    const nested = path.join(tmpDir, 'a', 'b', 'logs');
    const logger = createFileLogger({
      dir: nested,
      prefix: 'daemon',
      level: 'info',
      now: () => new Date('2026-05-19T00:00:00Z'),
    });
    logger.info({ module: 'x' }, 'mkdir');
    logger.close();
    expect(fs.existsSync(nested)).toBe(true);
    expect(fs.existsSync(path.join(nested, 'daemon-2026-05-19.log'))).toBe(true);
  });
});
