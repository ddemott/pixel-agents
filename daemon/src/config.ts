import * as fs from 'fs';

import { CONFIG_JSON_PATH } from './paths.js';

/**
 * Shape of `~/.pixel-agents/config.json` — kept in sync with the VS Code
 * extension's src/configPersistence.ts. New fields here must remain optional
 * so the extension can still read files the daemon writes.
 */
export interface PixelAgentsConfig {
  externalAssetDirectories: string[];
}

const DEFAULT_CONFIG: PixelAgentsConfig = {
  externalAssetDirectories: [],
};

/** Read config.json with the same field-by-field defensive parsing the extension uses. */
export function readConfig(): PixelAgentsConfig {
  try {
    if (!fs.existsSync(CONFIG_JSON_PATH)) return { ...DEFAULT_CONFIG };
    const raw = fs.readFileSync(CONFIG_JSON_PATH, 'utf-8');
    const parsed = JSON.parse(raw) as Partial<PixelAgentsConfig>;
    return {
      externalAssetDirectories: Array.isArray(parsed.externalAssetDirectories)
        ? parsed.externalAssetDirectories.filter((d): d is string => typeof d === 'string')
        : [],
    };
  } catch (err) {
    console.error('[Daemon] Failed to read config.json:', err);
    return { ...DEFAULT_CONFIG };
  }
}
