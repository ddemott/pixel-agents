import * as crypto from 'crypto';

const MAX_ID = 0x7fff_ffff; // 31-bit max; Kitty protocol i= field is u32

/**
 * Allocates stable Kitty graphics protocol image IDs.
 *
 * SHA1(assetId + '/' + tier + '/' + zoom) → 31-bit u32.
 * 0 is never emitted (reserved / invalid in the Kitty protocol).
 * Linear probe on collision — the set of live IDs is small in practice
 * (O(assets × tiers × zoom-levels)), so this never runs long.
 *
 * DEBT / RECONCILE (decision needed): this allocator is currently UNUSED. The
 * shipped wire path (`assets.requestBlob` here + the Rust client) keys Kitty
 * images by `djb2(assetId)` — one id per asset, no tier/zoom dimension. This
 * SHA1 allocator was built for a richer scheme (distinct ids per tier × zoom)
 * the protocol doesn't yet use. Pick during the Day 17 compositor: either wire
 * this over the wire and drop djb2 (gaining per-zoom image caching), or delete
 * it and keep djb2. Don't leave both indefinitely. Until then djb2 is the truth.
 */
export class KittyIdAllocator {
  /** key → allocated id */
  private readonly byKey = new Map<string, number>();
  /** allocated id → key (reverse map for collision probe) */
  private readonly byId = new Map<number, string>();

  allocate(assetId: string, tier: number, zoom: number): number {
    const key = `${assetId}/${tier}/${zoom}`;
    const existing = this.byKey.get(key);
    if (existing !== undefined) return existing;

    let id = hashToId(key);
    while (id === 0 || this.byId.has(id)) {
      id = (id + 1) & MAX_ID;
      if (id === 0) id = 1; // skip 0
    }

    this.byKey.set(key, id);
    this.byId.set(id, key);
    return id;
  }

  /** Remove a previously-allocated entry so its ID can be reused. */
  free(assetId: string, tier: number, zoom: number): void {
    const key = `${assetId}/${tier}/${zoom}`;
    const id = this.byKey.get(key);
    if (id !== undefined) {
      this.byKey.delete(key);
      this.byId.delete(id);
    }
  }

  size(): number {
    return this.byKey.size;
  }
}

function hashToId(key: string): number {
  const h = crypto.createHash('sha1').update(key).digest();
  // Read first 4 bytes as big-endian u32, mask to 31 bits.
  const raw = h.readUInt32BE(0);
  return raw & MAX_ID;
}
