import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { watchTagged } from '../../src/persistence/watcher.js';
import { type WriterTag, writeTagged } from '../../src/persistence/writerTag.js';

const OURS: WriterTag = { processId: 100, bootId: 'boot-A' };
const THEIRS: WriterTag = { processId: 999, bootId: 'boot-B' };

let tmpDir: string;
let file: string;

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-watch-'));
  file = path.join(tmpDir, 'data.json');
});

afterEach(() => {
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

function waitForChange(
  trigger: () => void,
  predicate: () => boolean,
  timeoutMs = 500,
): Promise<void> {
  trigger();
  return new Promise((resolve, reject) => {
    const start = Date.now();
    const tick = (): void => {
      if (predicate()) return resolve();
      if (Date.now() - start > timeoutMs) return reject(new Error('timeout'));
      setTimeout(tick, 20);
    };
    tick();
  });
}

describe('watchTagged', () => {
  it('emits on external writes', async () => {
    // Seed with our own write so the watcher has a baseline mtime.
    writeTagged(file, { v: 0 }, OURS);

    const received: Array<Record<string, unknown>> = [];
    const watcher = watchTagged<{ v: number }>(file, OURS, (data) => received.push(data), {
      pollIntervalMs: 50,
    });

    await waitForChange(
      () => {
        // Simulate another process writing — must have a different mtime.
        // Bump it manually to defeat sub-ms mtime resolution on some filesystems.
        const future = (Date.now() + 1000) / 1000;
        writeTagged(file, { v: 1 }, THEIRS);
        fs.utimesSync(file, future, future);
      },
      () => received.length > 0,
    );

    watcher.dispose();
    expect(received).toHaveLength(1);
    expect(received[0]).toEqual({ v: 1 });
  });

  it('ignores own-writes (matching bootId)', async () => {
    writeTagged(file, { v: 0 }, OURS);

    const received: Array<Record<string, unknown>> = [];
    const watcher = watchTagged<{ v: number }>(file, OURS, (data) => received.push(data), {
      pollIntervalMs: 50,
    });

    const future = (Date.now() + 1000) / 1000;
    writeTagged(file, { v: 1 }, OURS);
    fs.utimesSync(file, future, future);

    // Give the watcher ample time to (not) fire.
    await new Promise((r) => setTimeout(r, 200));
    watcher.dispose();
    expect(received).toHaveLength(0);
  });

  it('treats missing file as no-event until it appears, then notifies', async () => {
    const received: Array<Record<string, unknown>> = [];
    const watcher = watchTagged<{ v: number }>(file, OURS, (data) => received.push(data), {
      pollIntervalMs: 50,
    });

    await waitForChange(
      () => writeTagged(file, { v: 99 }, THEIRS),
      () => received.length > 0,
    );

    watcher.dispose();
    expect(received[0]).toEqual({ v: 99 });
  });

  it('survives a malformed file without crashing', async () => {
    writeTagged(file, { v: 0 }, OURS);
    const received: Array<Record<string, unknown>> = [];
    const watcher = watchTagged<{ v: number }>(file, OURS, (data) => received.push(data), {
      pollIntervalMs: 50,
    });

    // Corrupt the file.
    const future = (Date.now() + 1000) / 1000;
    fs.writeFileSync(file, '{not json');
    fs.utimesSync(file, future, future);

    // Then write valid external content.
    await waitForChange(
      () => {
        const later = (Date.now() + 2000) / 1000;
        writeTagged(file, { v: 5 }, THEIRS);
        fs.utimesSync(file, later, later);
      },
      () => received.length > 0,
    );

    watcher.dispose();
    expect(received[0]).toEqual({ v: 5 });
  });
});
