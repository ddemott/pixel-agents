import { LAYOUT_JSON_PATH } from '../paths.js';
import { type TaggedWatcher, watchTagged } from '../persistence/watcher.js';
import { readTagged, type WriterTag, writeTagged } from '../persistence/writerTag.js';

/**
 * Layout file persistence with writer-tag semantics (arch §16). The layout
 * shape itself is the unchanged `OfficeLayout` from `webview-ui/src/office/types.ts`
 * — kept as a loose record here so the daemon doesn't drag in the webview
 * type. Tag-handling lives in `persistence/writerTag.ts`; the watcher lives in
 * `persistence/watcher.ts`. This module is the thin layout-specific entry
 * point that the daemon boot wires up.
 *
 * Save coalescing (K5): client `layout.save` RPC payloads should be debounced
 * 500 ms before reaching `writeLayout`. The RPC catalog (Day 7-8) owns the
 * debounce; this module simply writes when asked.
 */

const DEBOUNCE_MS = 500;

export type Layout = Record<string, unknown>;

export function readLayout(): Layout | null {
  const result = readTagged<Layout>(LAYOUT_JSON_PATH);
  return result ? result.data : null;
}

export function writeLayout(layout: Layout, ours: WriterTag): void {
  writeTagged(LAYOUT_JSON_PATH, layout, ours);
}

export function watchLayout(ours: WriterTag, onExternal: (layout: Layout) => void): TaggedWatcher {
  return watchTagged<Layout>(LAYOUT_JSON_PATH, ours, (data) => onExternal(data));
}

/**
 * Lightweight debouncer the RPC layer uses to coalesce rapid client writes.
 * Exported here (rather than a generic util) so the layout module owns its own
 * write timing policy.
 */
export class LayoutSaveDebouncer {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private pending: Layout | null = null;

  constructor(
    private readonly ours: WriterTag,
    private readonly delayMs: number = DEBOUNCE_MS,
  ) {}

  schedule(layout: Layout): void {
    this.pending = layout;
    if (this.timer) return;
    this.timer = setTimeout(() => this.flushNow(), this.delayMs);
    this.timer.unref?.();
  }

  flushNow(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    if (this.pending) {
      writeLayout(this.pending, this.ours);
      this.pending = null;
    }
  }

  dispose(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.pending = null;
  }
}
