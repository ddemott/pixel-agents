import { describe, expect, it } from 'vitest';

import { PALETTE_CONSTANTS, pickDiversePalette } from '../../src/agents/palette.js';

/**
 * WHO: daemon agent.spawn palette assignment.
 * WHAT: pickDiversePalette mirrors the webview's diverse-skin logic.
 * WHY: every agent persisted palette:0 before — clones looked identical.
 */
describe('pickDiversePalette', () => {
  const { NUM_PALETTES, HUE_SHIFT_MIN_DEG, HUE_SHIFT_RANGE_DEG } = PALETTE_CONSTANTS;

  it('gives the first NUM_PALETTES agents unique skins with no hue shift', () => {
    const existing: { palette: number; hueShift: number }[] = [];
    const seen = new Set<number>();
    for (let i = 0; i < NUM_PALETTES; i++) {
      const pick = pickDiversePalette(existing);
      expect(pick.hueShift).toBe(0); // unique round → no rotation
      expect(seen.has(pick.palette)).toBe(false); // each palette once
      seen.add(pick.palette);
      existing.push({ palette: pick.palette, hueShift: pick.hueShift });
    }
    expect(seen.size).toBe(NUM_PALETTES);
  });

  it('hue-shifts once palettes start repeating (beyond NUM_PALETTES agents)', () => {
    // Seed one of every palette so the next pick must repeat.
    const existing = Array.from({ length: NUM_PALETTES }, (_, i) => ({ palette: i, hueShift: 0 }));
    const pick = pickDiversePalette(existing);
    expect(pick.palette).toBeGreaterThanOrEqual(0);
    expect(pick.palette).toBeLessThan(NUM_PALETTES);
    expect(pick.hueShift).toBeGreaterThanOrEqual(HUE_SHIFT_MIN_DEG);
    expect(pick.hueShift).toBeLessThan(HUE_SHIFT_MIN_DEG + HUE_SHIFT_RANGE_DEG);
  });

  it('picks the least-used palette deterministically with a stubbed rng', () => {
    // Palettes 0,1 used once; 2-5 unused → minCount 0, available = [2,3,4,5].
    const existing = [
      { palette: 0, hueShift: 0 },
      { palette: 1, hueShift: 0 },
    ];
    // rng()=0 → first available index → palette 2.
    const pick = pickDiversePalette(existing, () => 0);
    expect(pick.palette).toBe(2);
    expect(pick.hueShift).toBe(0);
  });

  it('ignores out-of-range palettes when counting', () => {
    // A bogus palette index must not crash or skew counts.
    const existing = [{ palette: 99, hueShift: 0 }];
    const pick = pickDiversePalette(existing, () => 0);
    expect(pick.palette).toBe(0); // all real palettes still at count 0
    expect(pick.hueShift).toBe(0);
  });
});
