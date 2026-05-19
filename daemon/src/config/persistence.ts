import { CONFIG_JSON_PATH } from '../paths.js';
import { type TaggedWatcher, watchTagged } from '../persistence/watcher.js';
import { readTagged, type WriterTag, writeTagged } from '../persistence/writerTag.js';

/**
 * config.json persistence. Same writer-tag pattern as layout.json. The shape
 * is kept structurally compatible with `src/configPersistence.ts` so the VS
 * Code extension and daemon can share the file during the transition. New
 * fields here must remain optional (default-friendly) for that reason.
 */

export interface PixelAgentsConfig {
  externalAssetDirectories: string[];
}

const DEFAULT_CONFIG: PixelAgentsConfig = {
  externalAssetDirectories: [],
};

/**
 * Defensive parse: hostile/legacy files yield the default rather than throwing.
 * Same per-field array filter the extension uses, so a config file written
 * with junk values still produces a safe in-memory shape.
 */
function coerce(parsed: Partial<PixelAgentsConfig>): PixelAgentsConfig {
  return {
    externalAssetDirectories: Array.isArray(parsed.externalAssetDirectories)
      ? parsed.externalAssetDirectories.filter((d): d is string => typeof d === 'string')
      : [],
  };
}

export function readConfig(): PixelAgentsConfig {
  const result = readTagged<Partial<PixelAgentsConfig>>(CONFIG_JSON_PATH);
  if (!result) return { ...DEFAULT_CONFIG };
  return coerce(result.data);
}

export function writeConfig(config: PixelAgentsConfig, ours: WriterTag): void {
  writeTagged(CONFIG_JSON_PATH, config as unknown as Record<string, unknown>, ours);
}

export function watchConfig(
  ours: WriterTag,
  onExternal: (config: PixelAgentsConfig) => void,
): TaggedWatcher {
  return watchTagged<Partial<PixelAgentsConfig>>(CONFIG_JSON_PATH, ours, (data) =>
    onExternal(coerce(data)),
  );
}
