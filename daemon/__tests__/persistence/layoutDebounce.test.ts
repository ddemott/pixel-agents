import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { readTagged, type WriterTag } from '../../src/persistence/writerTag.js';

const OURS: WriterTag = { processId: 1, bootId: 'boot-test' };

let tmpDir: string;
let originalHome: string | undefined;

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-layout-'));
  originalHome = process.env.HOME;
  process.env.HOME = tmpDir;
  vi.resetModules();
});

afterEach(() => {
  if (originalHome === undefined) delete process.env.HOME;
  else process.env.HOME = originalHome;
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

describe('LayoutSaveDebouncer', () => {
  it('coalesces multiple schedule() calls into a single write', async () => {
    // Use a short delay so the test finishes fast.
    const { LayoutSaveDebouncer } = await import('../../src/layout/persistence.js');
    const deb = new LayoutSaveDebouncer(OURS, 30);
    deb.schedule({ version: 1, cols: 10, rows: 10, tiles: [] });
    deb.schedule({ version: 1, cols: 20, rows: 20, tiles: [] });
    deb.schedule({ version: 1, cols: 30, rows: 30, tiles: [] });

    // Wait for the debounce to flush.
    await new Promise((r) => setTimeout(r, 100));

    const { LAYOUT_JSON_PATH } = await import('../../src/paths.js');
    const result = readTagged<{ cols: number }>(LAYOUT_JSON_PATH);
    expect(result).not.toBeNull();
    expect(result?.data.cols).toBe(30);
    expect(result?.tag).toEqual(OURS);
    deb.dispose();
  });

  it('flushNow() writes immediately without waiting for the timer', async () => {
    const { LayoutSaveDebouncer } = await import('../../src/layout/persistence.js');
    const deb = new LayoutSaveDebouncer(OURS, 10_000);
    deb.schedule({ version: 1, cols: 5, rows: 5, tiles: [] });
    deb.flushNow();
    const { LAYOUT_JSON_PATH } = await import('../../src/paths.js');
    const result = readTagged<{ cols: number }>(LAYOUT_JSON_PATH);
    expect(result?.data.cols).toBe(5);
    deb.dispose();
  });

  it('dispose() cancels a pending write', async () => {
    const { LayoutSaveDebouncer } = await import('../../src/layout/persistence.js');
    const deb = new LayoutSaveDebouncer(OURS, 50);
    deb.schedule({ version: 1, cols: 7, rows: 7, tiles: [] });
    deb.dispose();
    await new Promise((r) => setTimeout(r, 150));
    const { LAYOUT_JSON_PATH } = await import('../../src/paths.js');
    expect(fs.existsSync(LAYOUT_JSON_PATH)).toBe(false);
  });
});
