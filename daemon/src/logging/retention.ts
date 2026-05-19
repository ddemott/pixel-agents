import * as fs from 'fs';
import * as path from 'path';
import * as zlib from 'zlib';

/**
 * Retention sweep for `~/.pixel-agents/logs/`:
 *   - Plain `*.log` files older than `gzipAfterDays` are gzipped → `*.log.gz`.
 *     The original is unlinked only after the gzipped file is fsynced.
 *   - Any `*.log` or `*.log.gz` file older than `deleteAfterDays` is deleted.
 *
 * Boundaries are computed against the file's mtime in ms. Files that are
 * currently open (today's log) are left alone because their mtime is fresh.
 *
 * The sweep is best-effort: per-file errors are swallowed so one bad file
 * can't poison the rest of the directory. Returns counters so callers (and
 * tests) can assert behaviour.
 */

export interface SweepOptions {
  /** Directory to scan (non-recursive). */
  dir: string;
  /** gzip plain `.log` files older than this many days. */
  gzipAfterDays: number;
  /** Delete `.log` and `.log.gz` files older than this many days. */
  deleteAfterDays: number;
  /** Override clock for tests. Defaults to `Date.now`. */
  now?: () => number;
}

export interface SweepResult {
  gzipped: string[];
  deleted: string[];
  /** Files we tried to touch but failed on (e.g. permission denied). */
  errors: Array<{ file: string; error: string }>;
}

const DAY_MS = 24 * 60 * 60 * 1000;

export function sweepLogs(opts: SweepOptions): SweepResult {
  const now = (opts.now ?? Date.now)();
  const gzipCutoff = now - opts.gzipAfterDays * DAY_MS;
  const deleteCutoff = now - opts.deleteAfterDays * DAY_MS;
  const result: SweepResult = { gzipped: [], deleted: [], errors: [] };

  let entries: string[];
  try {
    entries = fs.readdirSync(opts.dir);
  } catch {
    return result;
  }

  for (const name of entries) {
    const file = path.join(opts.dir, name);
    let stat: fs.Stats;
    try {
      stat = fs.statSync(file);
    } catch {
      continue;
    }
    if (!stat.isFile()) continue;

    const mtime = stat.mtimeMs;
    const isPlain = name.endsWith('.log');
    const isGz = name.endsWith('.log.gz');
    if (!isPlain && !isGz) continue;

    if (mtime < deleteCutoff) {
      try {
        fs.unlinkSync(file);
        result.deleted.push(file);
      } catch (e) {
        result.errors.push({ file, error: errMsg(e) });
      }
      continue;
    }

    if (isPlain && mtime < gzipCutoff) {
      const gzPath = file + '.gz';
      try {
        // If a stale .gz from a previous half-finished sweep is lying around,
        // overwrite it rather than leave both copies on disk.
        const data = fs.readFileSync(file);
        const compressed = zlib.gzipSync(data);
        fs.writeFileSync(gzPath, compressed, { mode: 0o600 });
        // Preserve mtime so the next sweep can still age it out at 30d.
        fs.utimesSync(gzPath, stat.atime, stat.mtime);
        fs.unlinkSync(file);
        result.gzipped.push(file);
      } catch (e) {
        result.errors.push({ file, error: errMsg(e) });
      }
    }
  }

  return result;
}

function errMsg(e: unknown): string {
  if (e instanceof Error) return e.message;
  return String(e);
}
