import { describe, expect, it } from 'vitest';

import { mergeConfig } from '../../src/configPersistence.js';

/**
 * WHO: VS Code extension's config writer (src/configPersistence.ts).
 * WHAT: writeConfig used to serialize only its own fields, dropping keys it
 *       doesn't know about — notably the daemon's logLevel.
 * WHY: extension + daemon both write ~/.pixel-agents/config.json; a naive
 *      overwrite silently reset the daemon's log level on every extension save.
 * mergeConfig is the pure core of the read-merge-write fix.
 */
describe('mergeConfig', () => {
  it('preserves daemon-owned keys (logLevel) the extension does not model', () => {
    const existing = { externalAssetDirectories: [], logLevel: 'debug' };
    const merged = mergeConfig(existing, { externalAssetDirectories: ['/packs'] });
    expect(merged.logLevel).toBe('debug'); // survives
    expect(merged.externalAssetDirectories).toEqual(['/packs']); // updated
  });

  it('overwrites only the extension-owned field', () => {
    const existing = { externalAssetDirectories: ['/old'], somethingFuture: 42 };
    const merged = mergeConfig(existing, { externalAssetDirectories: ['/new'] });
    expect(merged.externalAssetDirectories).toEqual(['/new']);
    expect(merged.somethingFuture).toBe(42);
  });

  it('works from an empty existing config', () => {
    const merged = mergeConfig({}, { externalAssetDirectories: ['/x'] });
    expect(merged).toEqual({ externalAssetDirectories: ['/x'] });
  });
});
