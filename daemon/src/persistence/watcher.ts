import * as fs from 'fs';

import { isOwnWrite, readTagged, type WriterTag } from './writerTag.js';

/**
 * Hybrid file watcher: `fs.watch` for instant notifications + interval poll as
 * a backup (Windows kqueue + inotify limits make `fs.watch` unreliable on its
 * own, and arch §16 explicitly requires both legs).
 *
 * Reads the file as JSON-with-writer-tag. Skips emitting when the writer tag
 * matches the daemon's own `ours` tag (own-write), and emits the parsed,
 * untagged data otherwise.
 */

const DEFAULT_POLL_INTERVAL_MS = 2_000;

export interface TaggedWatcher {
  dispose(): void;
}

export interface WatchOptions {
  /** Polling fallback interval. Defaults to 2 s. */
  pollIntervalMs?: number;
}

export function watchTagged<T extends Record<string, unknown>>(
  filePath: string,
  ours: WriterTag,
  onExternal: (data: T, tag: WriterTag | null) => void,
  opts: WatchOptions = {},
): TaggedWatcher {
  const pollIntervalMs = opts.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
  let lastMtime = readMtime(filePath);
  let fsWatcher: fs.FSWatcher | null = null;
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let disposed = false;

  const check = (): void => {
    if (disposed) return;
    const mtime = readMtime(filePath);
    if (mtime === 0 || mtime <= lastMtime) return;
    lastMtime = mtime;
    const result = readTagged<T>(filePath);
    if (!result) return; // malformed or missing — wait for the next write
    if (isOwnWrite(result.tag, ours)) return;
    onExternal(result.data, result.tag);
  };

  const startFsWatch = (): void => {
    if (disposed || fsWatcher) return;
    if (!fs.existsSync(filePath)) return;
    try {
      fsWatcher = fs.watch(filePath, () => check());
      fsWatcher.on('error', () => {
        // fs.watch is best-effort. Polling is the safety net.
        fsWatcher?.close();
        fsWatcher = null;
      });
    } catch {
      // File may not exist yet — polling will retry.
    }
  };

  startFsWatch();
  pollTimer = setInterval(() => {
    if (disposed) return;
    if (!fsWatcher) startFsWatch();
    check();
  }, pollIntervalMs);
  pollTimer.unref?.();

  return {
    dispose(): void {
      disposed = true;
      fsWatcher?.close();
      fsWatcher = null;
      if (pollTimer) {
        clearInterval(pollTimer);
        pollTimer = null;
      }
    },
  };
}

function readMtime(filePath: string): number {
  try {
    return fs.statSync(filePath).mtimeMs;
  } catch {
    return 0;
  }
}
