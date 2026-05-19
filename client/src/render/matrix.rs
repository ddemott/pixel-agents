#![allow(dead_code)]

use rand::Rng;

/// Generate 16 per-column random seed values for the matrix spawn/despawn effect.
/// The seeds control per-column stagger timing — full rendering lives in Day 20.
pub fn matrix_effect_seeds(rng: &mut impl Rng) -> [f32; 16] {
    let mut seeds = [0.0f32; 16];
    for s in &mut seeds {
        *s = rng.gen();
    }
    seeds
}
