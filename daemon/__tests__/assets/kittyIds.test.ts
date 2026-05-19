import { describe, expect, it } from 'vitest';

import { KittyIdAllocator } from '../../src/assets/kittyIds.js';

describe('KittyIdAllocator', () => {
  it('returns same id for same key', () => {
    const alloc = new KittyIdAllocator();
    const a = alloc.allocate('DESK_FRONT', 0, 2);
    const b = alloc.allocate('DESK_FRONT', 0, 2);
    expect(a).toBe(b);
  });

  it('returns different ids for different keys', () => {
    const alloc = new KittyIdAllocator();
    const a = alloc.allocate('DESK_FRONT', 0, 2);
    const b = alloc.allocate('DESK_BACK', 0, 2);
    expect(a).not.toBe(b);
  });

  it('never allocates id 0', () => {
    const alloc = new KittyIdAllocator();
    for (let i = 0; i < 1000; i++) {
      const id = alloc.allocate(`asset_${i}`, 0, 1);
      expect(id).not.toBe(0);
    }
  });

  it('ids are 31-bit (≤ 0x7fffffff)', () => {
    const alloc = new KittyIdAllocator();
    for (let i = 0; i < 100; i++) {
      const id = alloc.allocate(`asset_${i}`, i, i % 4);
      expect(id).toBeGreaterThan(0);
      expect(id).toBeLessThanOrEqual(0x7fff_ffff);
    }
  });

  it('tier and zoom are part of the key', () => {
    const alloc = new KittyIdAllocator();
    const a = alloc.allocate('DESK_FRONT', 0, 1);
    const b = alloc.allocate('DESK_FRONT', 1, 1);
    const c = alloc.allocate('DESK_FRONT', 0, 2);
    expect(a).not.toBe(b);
    expect(a).not.toBe(c);
    expect(b).not.toBe(c);
  });

  it('free allows id reuse', () => {
    const alloc = new KittyIdAllocator();
    const id1 = alloc.allocate('DESK_FRONT', 0, 2);
    expect(alloc.size()).toBe(1);
    alloc.free('DESK_FRONT', 0, 2);
    expect(alloc.size()).toBe(0);
    const id2 = alloc.allocate('DESK_FRONT', 0, 2);
    expect(id2).toBe(id1); // same hash → same id
  });

  it('survives hash collision via linear probe', () => {
    // Force a collision by pre-occupying the id that would be allocated.
    // We can't easily predict the hash, so just allocate a large batch
    // and confirm all are unique and non-zero.
    const alloc = new KittyIdAllocator();
    const ids = new Set<number>();
    for (let i = 0; i < 500; i++) {
      const id = alloc.allocate(`collision_test_${i}`, 0, 0);
      expect(ids.has(id)).toBe(false);
      ids.add(id);
    }
  });
});
