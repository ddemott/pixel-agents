import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import * as zlib from 'zlib';

import { sweepLogs } from '../../src/logging/retention.js';

let tmpDir: string;
const NOW = new Date('2026-05-19T00:00:00Z').getTime();
const DAY = 24 * 60 * 60 * 1000;

function touchFile(name: string, mtimeMs: number, contents = 'data'): string {
  const file = path.join(tmpDir, name);
  fs.writeFileSync(file, contents);
  const sec = mtimeMs / 1000;
  fs.utimesSync(file, sec, sec);
  return file;
}

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-retention-'));
});

afterEach(() => {
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

describe('sweepLogs', () => {
  it('leaves fresh logs alone', () => {
    const file = touchFile('daemon-2026-05-19.log', NOW - 1 * DAY);
    const res = sweepLogs({
      dir: tmpDir,
      gzipAfterDays: 7,
      deleteAfterDays: 30,
      now: () => NOW,
    });
    expect(res.gzipped).toEqual([]);
    expect(res.deleted).toEqual([]);
    expect(fs.existsSync(file)).toBe(true);
  });

  it('gzips plain .log files older than the gzip cutoff', () => {
    const file = touchFile('daemon-2026-05-10.log', NOW - 8 * DAY, 'old data');
    const res = sweepLogs({
      dir: tmpDir,
      gzipAfterDays: 7,
      deleteAfterDays: 30,
      now: () => NOW,
    });
    expect(res.gzipped).toEqual([file]);
    expect(fs.existsSync(file)).toBe(false);
    const gz = file + '.gz';
    expect(fs.existsSync(gz)).toBe(true);
    expect(zlib.gunzipSync(fs.readFileSync(gz)).toString('utf-8')).toBe('old data');
  });

  it('preserves the original mtime on the gzipped copy', () => {
    const oldMtime = NOW - 8 * DAY;
    const file = touchFile('daemon-2026-05-10.log', oldMtime);
    sweepLogs({
      dir: tmpDir,
      gzipAfterDays: 7,
      deleteAfterDays: 30,
      now: () => NOW,
    });
    const gz = file + '.gz';
    const stat = fs.statSync(gz);
    // Allow 2s rounding on filesystems that truncate utimes (FAT, some HFS+).
    expect(Math.abs(stat.mtimeMs - oldMtime)).toBeLessThan(2000);
  });

  it('deletes .log and .log.gz older than the delete cutoff', () => {
    const oldLog = touchFile('daemon-2026-04-01.log', NOW - 40 * DAY);
    const oldGz = touchFile('daemon-2026-04-02.log.gz', NOW - 40 * DAY);
    const res = sweepLogs({
      dir: tmpDir,
      gzipAfterDays: 7,
      deleteAfterDays: 30,
      now: () => NOW,
    });
    expect(res.deleted.sort()).toEqual([oldLog, oldGz].sort());
    expect(fs.existsSync(oldLog)).toBe(false);
    expect(fs.existsSync(oldGz)).toBe(false);
  });

  it('ignores files that are not .log / .log.gz', () => {
    const other = touchFile('config.json', NOW - 99 * DAY);
    const res = sweepLogs({
      dir: tmpDir,
      gzipAfterDays: 7,
      deleteAfterDays: 30,
      now: () => NOW,
    });
    expect(res.gzipped).toEqual([]);
    expect(res.deleted).toEqual([]);
    expect(fs.existsSync(other)).toBe(true);
  });

  it('handles a missing directory without throwing', () => {
    const res = sweepLogs({
      dir: path.join(tmpDir, 'missing'),
      gzipAfterDays: 7,
      deleteAfterDays: 30,
      now: () => NOW,
    });
    expect(res).toEqual({ gzipped: [], deleted: [], errors: [] });
  });
});
