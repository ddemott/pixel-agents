import * as fs from 'fs';
import * as path from 'path';

/**
 * Writer-tag scheme from docs/tui-architecture.md §16. Every file persisted by
 * the daemon carries a `_writer` field at the end of the JSON payload:
 *
 *   "_writer": { "processId": 12345, "bootId": "<uuid>" }
 *
 * The bootId is regenerated on every daemon start, so a file watcher can tell
 * "our own write" (matching bootId) from "external write" (different bootId,
 * e.g. another daemon process, manual edit, or the previous instance of this
 * daemon before a restart). This is robust to filesystem clock drift and
 * concurrent writes — unlike the v1 timestamp-proximity heuristic.
 */

export interface WriterTag {
  processId: number;
  bootId: string;
}

const WRITER_FIELD = '_writer';

/**
 * Read a JSON file and extract its writer tag, if present. Returns `{ data, tag }`
 * with the writer field stripped from `data`. Returns null when the file is
 * missing or malformed (caller decides how to recover).
 */
export function readTagged<T extends Record<string, unknown>>(
  filePath: string,
): {
  data: T;
  tag: WriterTag | null;
} | null {
  try {
    if (!fs.existsSync(filePath)) return null;
    const raw = fs.readFileSync(filePath, 'utf-8');
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const tagRaw = parsed[WRITER_FIELD];
    const tag = isWriterTag(tagRaw) ? tagRaw : null;
    // Strip the tag from the returned payload so callers don't have to know
    // about it.
    const data = { ...parsed };
    delete data[WRITER_FIELD];
    return { data: data as T, tag };
  } catch {
    return null;
  }
}

/**
 * Atomic write of `data` plus a writer tag. Writes to `<filePath>.tmp` then
 * renames into place. The renamed file inherits the tmp file's mode (0o600).
 */
export function writeTagged(filePath: string, data: Record<string, unknown>, tag: WriterTag): void {
  const dir = path.dirname(filePath);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  }
  const payload = { ...data, [WRITER_FIELD]: tag };
  const tmp = filePath + '.tmp';
  fs.writeFileSync(tmp, JSON.stringify(payload, null, 2), { mode: 0o600 });
  fs.renameSync(tmp, filePath);
}

/** True when `tag` is present and belongs to the daemon identified by `ours`. */
export function isOwnWrite(tag: WriterTag | null, ours: WriterTag): boolean {
  return tag !== null && tag.bootId === ours.bootId;
}

function isWriterTag(value: unknown): value is WriterTag {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as WriterTag).processId === 'number' &&
    typeof (value as WriterTag).bootId === 'string'
  );
}
