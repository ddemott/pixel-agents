import * as fs from 'fs';

import type { AssetRegistry } from './registry.js';

const DEBOUNCE_MS = 250;

/**
 * Watch an asset directory for changes and trigger registry reload on change.
 * Uses `fs.watch` + 250ms debounce (same pattern as persistence/watcher.ts).
 * Returns a stop function.
 */
export function watchAssetDir(dir: string, registry: AssetRegistry): () => void {
  if (!fs.existsSync(dir)) return () => {};

  let timer: ReturnType<typeof setTimeout> | null = null;
  let watcher: fs.FSWatcher | null = null;

  const schedule = (): void => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      registry.reload('fs.watch');
    }, DEBOUNCE_MS);
  };

  try {
    watcher = fs.watch(dir, { recursive: true }, (_event, _filename) => {
      schedule();
    });
    watcher.on('error', () => {
      // Best effort — suppress errors; polling fallback not needed for asset dirs
    });
  } catch {
    // fs.watch unavailable on this platform for this path — no-op
  }

  return () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    watcher?.close();
    watcher = null;
  };
}

/**
 * Watch all directories tracked by the registry (bundled + external).
 * Returns a combined stop function.
 */
export function watchAllAssetDirs(dirs: string[], registry: AssetRegistry): () => void {
  const stops = dirs.map((dir) => watchAssetDir(dir, registry));
  return () => stops.forEach((s) => s());
}
