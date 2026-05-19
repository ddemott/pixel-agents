import { CONFIG_JSON_PATH } from '../paths.js';
import { type TaggedWatcher, watchTagged } from '../persistence/watcher.js';
import { readTagged, type WriterTag, writeTagged } from '../persistence/writerTag.js';

/**
 * config.json persistence. Same writer-tag pattern as layout.json. The shape
 * is kept structurally compatible with `src/configPersistence.ts` so the VS
 * Code extension and daemon can share the file during the transition. New
 * fields here must remain optional (default-friendly) for that reason.
 */

/** Ordered low → high; see `logging/logger.ts`. */
export const LOG_LEVELS = ['trace', 'debug', 'info', 'warn', 'error'] as const;
export type LogLevel = (typeof LOG_LEVELS)[number];

export interface PixelAgentsConfig {
  externalAssetDirectories: string[];
  /**
   * Minimum log level the daemon writes to `~/.pixel-agents/logs/`. Below this
   * level, calls are dropped. Optional in the on-disk file (default `info`)
   * so old config.json values keep working. Owned by the daemon; the VS Code
   * extension currently ignores it.
   */
  logLevel: LogLevel;
}

const DEFAULT_CONFIG: PixelAgentsConfig = {
  externalAssetDirectories: [],
  logLevel: 'info',
};

function isLogLevel(v: unknown): v is LogLevel {
  return typeof v === 'string' && (LOG_LEVELS as readonly string[]).includes(v);
}

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
    logLevel: isLogLevel(parsed.logLevel) ? parsed.logLevel : DEFAULT_CONFIG.logLevel,
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
