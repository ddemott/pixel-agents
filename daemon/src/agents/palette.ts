/**
 * Diverse palette assignment for newly-spawned agents — daemon port of the
 * webview's `OfficeState.pickDiversePalette()`.
 *
 * Counts the base palette (0-5) of existing agents and picks randomly from the
 * least-used palette(s). The first `NUM_PALETTES` agents each get a unique skin
 * (minCount stays 0 → no hue shift); beyond that, palettes repeat with a random
 * hue rotation (≥45°) so duplicates stay visually distinct.
 */

const NUM_PALETTES = 6;
const HUE_SHIFT_MIN_DEG = 45;
const HUE_SHIFT_RANGE_DEG = 271;

/** Anything carrying a base palette index — `PersistedAgent` qualifies. */
interface HasPalette {
  palette: number;
}

export function pickDiversePalette(
  existing: readonly HasPalette[],
  rng: () => number = Math.random,
): { palette: number; hueShift: number } {
  const counts = new Array<number>(NUM_PALETTES).fill(0);
  for (const a of existing) {
    if (a.palette >= 0 && a.palette < NUM_PALETTES) counts[a.palette]++;
  }

  const minCount = Math.min(...counts);
  const available: number[] = [];
  for (let i = 0; i < NUM_PALETTES; i++) {
    if (counts[i] === minCount) available.push(i);
  }

  const palette = available[Math.floor(rng() * available.length)];
  // First round (minCount === 0): unique skins, no hue shift. After that,
  // repeats get a random rotation in [HUE_SHIFT_MIN_DEG, +RANGE).
  const hueShift = minCount > 0 ? HUE_SHIFT_MIN_DEG + Math.floor(rng() * HUE_SHIFT_RANGE_DEG) : 0;

  return { palette, hueShift };
}

export const PALETTE_CONSTANTS = { NUM_PALETTES, HUE_SHIFT_MIN_DEG, HUE_SHIFT_RANGE_DEG };
