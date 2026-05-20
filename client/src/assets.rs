//! Client-side asset blob ingestion (Phase 3 Day 10).
//!
//! The daemon ships sprite PNGs over the 0x02 asset channel in response to an
//! `assets.requestBlob { assetId, tier }` RPC. Each frame carries a *numeric*
//! asset id — `djb2(assetId)` (see `stringAssetId` in the daemon's
//! `rpc/methods/agents.ts`) — plus a tier byte and an `is_final` flag. We
//! reassemble the chunks per `(numeric_id, tier)` and decode the completed PNG
//! to RGBA8 for the renderer's sprite cache.

use std::collections::HashMap;

use anyhow::{anyhow, Result};

/// djb2 string hash, matching the daemon's `stringAssetId()` exactly:
/// `h = 5381; for c in id: h = (h*33 + c) mod 2^32`. Operates on the string's
/// bytes; asset ids are ASCII so this agrees with the daemon's `charCodeAt`.
pub fn string_asset_id(id: &str) -> u32 {
    let mut h: u32 = 5381;
    for b in id.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u32);
    }
    h
}

/// A decoded sprite: tightly-packed RGBA8, row-major, `width * height * 4` bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecodedAsset {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Accumulates streamed asset frames and exposes decoded sprites by string id.
#[derive(Default)]
pub struct AssetStore {
    /// numeric id (djb2) → string asset id, populated when we issue a request.
    by_numeric: HashMap<u32, String>,
    /// (numeric id, tier) → bytes accumulated so far.
    pending: HashMap<(u32, u8), Vec<u8>>,
    /// string asset id → decoded RGBA sprite.
    decoded: HashMap<String, DecodedAsset>,
}

impl AssetStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that we've requested `asset_id`; returns the numeric id the daemon
    /// will tag the resulting frames with. Call before sending `assets.requestBlob`.
    pub fn register_request(&mut self, asset_id: &str) -> u32 {
        let numeric = string_asset_id(asset_id);
        self.by_numeric.insert(numeric, asset_id.to_string());
        numeric
    }

    /// Feed one asset frame. On the final chunk, decodes the PNG and stores it;
    /// returns `Ok(Some(asset_id))` naming the sprite that just completed, or
    /// `Ok(None)` while more chunks are pending. Unknown numeric ids (no prior
    /// `register_request`) are accumulated but decoded under a synthetic key, so
    /// nothing is silently dropped.
    pub fn on_frame(
        &mut self,
        numeric_id: u32,
        tier: u8,
        is_final: bool,
        bytes: &[u8],
    ) -> Result<Option<String>> {
        let entry = self.pending.entry((numeric_id, tier)).or_default();
        entry.extend_from_slice(bytes);

        if !is_final {
            return Ok(None);
        }

        let buf = self
            .pending
            .remove(&(numeric_id, tier))
            .expect("entry inserted above");

        let asset_id = self
            .by_numeric
            .get(&numeric_id)
            .cloned()
            .unwrap_or_else(|| format!("#{numeric_id}"));

        let decoded = decode_png(&buf)
            .map_err(|e| anyhow!("decode asset '{asset_id}' (tier {tier}): {e}"))?;
        self.decoded.insert(asset_id.clone(), decoded);
        Ok(Some(asset_id))
    }

    pub fn get(&self, asset_id: &str) -> Option<&DecodedAsset> {
        self.decoded.get(asset_id)
    }

    /// Iterate decoded sprites as `(asset_id, decoded)`. Order is unspecified.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &DecodedAsset)> {
        self.decoded.iter()
    }

    pub fn contains(&self, asset_id: &str) -> bool {
        self.decoded.contains_key(asset_id)
    }

    pub fn len(&self) -> usize {
        self.decoded.len()
    }

    pub fn is_empty(&self) -> bool {
        self.decoded.is_empty()
    }
}

fn decode_png(bytes: &[u8]) -> Result<DecodedAsset> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?;
    let rgba = img.to_rgba8();
    Ok(DecodedAsset {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn djb2_matches_daemon_vectors() {
        // Hand-computed against h = h*33 + c (mod 2^32), seed 5381.
        assert_eq!(string_asset_id(""), 5381);
        assert_eq!(string_asset_id("a"), 177670);
        assert_eq!(string_asset_id("abc"), 193485963);
    }

    /// Encode a solid `w×h` RGBA image to PNG for round-trip tests.
    fn make_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut buf = image::RgbaImage::new(w, h);
        for px in buf.pixels_mut() {
            *px = image::Rgba(rgba);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(buf)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn single_frame_decodes() {
        let png = make_png(2, 3, [10, 20, 30, 255]);
        let mut store = AssetStore::new();
        let nid = store.register_request("DESK");
        let done = store.on_frame(nid, 0, true, &png).unwrap();
        assert_eq!(done.as_deref(), Some("DESK"));
        let a = store.get("DESK").unwrap();
        assert_eq!((a.width, a.height), (2, 3));
        assert_eq!(a.rgba.len(), 2 * 3 * 4);
        assert_eq!(&a.rgba[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn multi_chunk_reassembles() {
        let png = make_png(4, 4, [1, 2, 3, 255]);
        let mid = png.len() / 2;
        let mut store = AssetStore::new();
        let nid = store.register_request("CHAIR");

        assert_eq!(store.on_frame(nid, 0, false, &png[..mid]).unwrap(), None);
        let done = store.on_frame(nid, 0, true, &png[mid..]).unwrap();
        assert_eq!(done.as_deref(), Some("CHAIR"));
        assert_eq!(store.get("CHAIR").unwrap().width, 4);
    }

    #[test]
    fn distinct_tiers_accumulate_independently() {
        let png0 = make_png(1, 1, [9, 9, 9, 255]);
        let png1 = make_png(2, 2, [8, 8, 8, 255]);
        let mut store = AssetStore::new();
        let nid = store.register_request("LAMP");

        // Interleave two tiers of the same asset; each must reassemble cleanly.
        assert_eq!(store.on_frame(nid, 0, false, &png0[..3]).unwrap(), None);
        assert_eq!(store.on_frame(nid, 1, false, &png1[..3]).unwrap(), None);
        store.on_frame(nid, 0, true, &png0[3..]).unwrap();
        store.on_frame(nid, 1, true, &png1[3..]).unwrap();
        // Tier 1 finished last, so its dims win the (single-keyed) store slot.
        assert_eq!(store.get("LAMP").unwrap().width, 2);
    }

    #[test]
    fn unknown_numeric_id_uses_synthetic_key() {
        let png = make_png(1, 1, [0, 0, 0, 255]);
        let mut store = AssetStore::new();
        let done = store.on_frame(12345, 0, true, &png).unwrap();
        assert_eq!(done.as_deref(), Some("#12345"));
    }

    #[test]
    fn corrupt_png_errors() {
        let mut store = AssetStore::new();
        let nid = store.register_request("BAD");
        assert!(store.on_frame(nid, 0, true, b"not a png").is_err());
    }
}
