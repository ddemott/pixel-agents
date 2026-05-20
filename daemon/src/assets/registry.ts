import * as fs from 'fs';
import * as path from 'path';

import type { BroadcastSink } from '../agents/broadcastSink.js';
import type { Logger } from '../logging/logger.js';
import {
  buildGroups,
  type BuiltCatalog,
  type FurnitureAsset,
  scanFurnitureDir,
} from './catalog.js';

export interface AssetDirs {
  /** Absolute path to the bundled furniture directory. */
  bundled: string;
  /** User-configured external directories (order preserved, later entries
   *  override earlier on ID collision). */
  external: string[];
}

export interface LoadedAssets {
  catalog: BuiltCatalog;
  /** assetId → raw PNG bytes (lazy-loaded on first `assets.requestBlob` hit) */
  pngCache: Map<string, Buffer>;
  /** assetId → absolute file path (for lazy load) */
  filePaths: Map<string, string>;
}

/**
 * Daemon-wide asset registry. Owns the merged furniture catalog built from
 * bundled + external directories. PNG bytes are loaded lazily and cached.
 *
 * `reload()` re-scans all directories and rebuilds the merged catalog.
 * It is called at boot and by the directory watcher on change.
 */
export class AssetRegistry {
  private dirs: AssetDirs;
  private loaded: LoadedAssets;
  private sink: BroadcastSink | null = null;
  private logger: Logger | null = null;

  constructor(dirs: AssetDirs) {
    this.dirs = dirs;
    this.loaded = this.build();
  }

  setSink(sink: BroadcastSink): void {
    this.sink = sink;
  }

  setLogger(logger: Logger): void {
    this.logger = logger;
  }

  getCatalog(): BuiltCatalog {
    return this.loaded.catalog;
  }

  getAssets(): FurnitureAsset[] {
    return this.loaded.catalog.assets;
  }

  /** Returns raw PNG bytes for `assetId`, lazily loaded from disk. */
  getPng(assetId: string): Buffer | null {
    const cached = this.loaded.pngCache.get(assetId);
    if (cached) return cached;

    const filePath = this.loaded.filePaths.get(assetId);
    if (!filePath) return null;

    try {
      const buf = fs.readFileSync(filePath);
      this.loaded.pngCache.set(assetId, buf);
      return buf;
    } catch {
      return null;
    }
  }

  updateExternalDirs(dirs: string[]): void {
    this.dirs = { ...this.dirs, external: dirs };
    this.reload('config');
  }

  reload(reason: string): void {
    this.logger?.info({ module: 'assets', reason }, 'reloading asset registry');
    this.loaded = this.build();
    this.sink?.post({ type: 'assets.updated', assetCount: this.loaded.catalog.assets.length });
  }

  private build(): LoadedAssets {
    // Bundled assets first; external directories overlay in order.
    const allDirs = [this.dirs.bundled, ...this.dirs.external];
    const assetsByDir: FurnitureAsset[][] = [];

    for (const dir of allDirs) {
      assetsByDir.push(scanFurnitureDir(dir));
    }

    // Merge: later dirs override same ID from earlier dirs.
    const merged = new Map<string, FurnitureAsset>();
    const filePaths = new Map<string, string>();

    for (let i = 0; i < allDirs.length; i++) {
      const dir = allDirs[i]!;
      const assets = assetsByDir[i]!;
      for (const asset of assets) {
        merged.set(asset.id, asset);
        // Resolve file path: furniture subdirectory named by groupId (or id prefix before _)
        const groupId = asset.groupId ?? asset.id.split('_')[0]!;
        filePaths.set(asset.id, path.join(dir, groupId, asset.file));
      }
    }

    // Character sprite sheets (Day 18): `char_N.png` live in the `characters`
    // sibling of the bundled furniture dir, not inside the furniture catalog.
    // Register them by bare id `char_N` so `assets.requestBlob { assetId:
    // "char_0" }` resolves the raw 112×96 sheet. External character packs are a
    // follow-up — the client falls back to placeholder blocks for missing ids.
    const charsDir = path.join(path.dirname(this.dirs.bundled), 'characters');
    try {
      for (const entry of fs.readdirSync(charsDir)) {
        const m = /^(char_\d+)\.png$/i.exec(entry);
        if (m) filePaths.set(m[1]!, path.join(charsDir, entry));
      }
    } catch {
      // characters dir absent (some packs) — fine, client uses placeholders.
    }

    const catalog = buildGroups([...merged.values()]);
    return { catalog, pngCache: new Map(), filePaths };
  }
}
