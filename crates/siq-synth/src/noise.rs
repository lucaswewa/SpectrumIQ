//! The deterministic randomness every world is made of: hashes of lattice points, value noise, and
//! fractal noise that fades out detail finer than the sampling.

use siq_types::Vec2Um;

/// SplitMix64's finaliser: a well-mixed 64-bit hash.
const fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// The key of one use of randomness, `salt`, in the world of `seed`. Worlds make their keys once.
pub(crate) const fn key(seed: u64, salt: u64) -> u64 {
    mix(seed ^ mix(salt))
}

/// The hash of lattice point `(i, j, k)` under `key`: one mix of the point's coordinates, each
/// times its own large odd constant. Points collide only when their coordinates are about 2⁶⁰
/// apart.
pub(crate) const fn hash(key: u64, i: i64, j: i64, k: i64) -> u64 {
    let i = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let j = (j as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    let k = (k as u64).wrapping_mul(0x1656_67b1_9e37_79f9);
    mix(key.wrapping_add(i).wrapping_add(j).wrapping_add(k))
}

/// A number in `[0, 1)` from a hash.
pub(crate) fn unit(hash: u64) -> f64 {
    (hash >> 11) as f64 / (1u64 << 53) as f64
}

/// `N` numbers in `[0, 1)` for lattice point `(i, j, k)` under `key`.
pub(crate) fn units<const N: usize>(key: u64, i: i64, j: i64, k: i64) -> [f64; N] {
    let base = hash(key, i, j, k);
    std::array::from_fn(|n| unit(mix(base ^ n as u64)))
}

/// The lattice cell of `value`, at `spacing`, and the position within it, from 0 to 1.
pub(crate) fn cell(value: f64, spacing: f64) -> (i64, f64) {
    let scaled = value / spacing;
    let floor = scaled.floor();
    (floor as i64, scaled - floor)
}

/// The lattice cells, at `spacing`, that hold the points within `reach` of `value`.
pub(crate) fn cells_within(value: f64, reach: f64, spacing: f64) -> std::ops::RangeInclusive<i64> {
    cell(value - reach, spacing).0..=cell(value + reach, spacing).0
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// `a`, moved toward `b` by `t`.
pub(crate) fn mix_values(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Value noise from 0 to 1, with features about `period` µm apart.
pub(crate) fn value(key: u64, at: Vec2Um, period: f64) -> f64 {
    let [x, y] = at.to_um();
    let ((i, fx), (j, fy)) = (cell(x, period), cell(y, period));
    let corner = |di, dj| unit(hash(key, i + di, j + dj, 0));
    let (fx, fy) = (smooth(fx), smooth(fy));
    let top = mix_values(corner(0, 0), corner(1, 0), fx);
    mix_values(top, mix_values(corner(0, 1), corner(1, 1), fx), fy)
}

/// One octave of fractal noise: its key, and the rotation that keeps its lattice from lining up
/// with the others'.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Octave {
    key: u64,
    cos: f64,
    sin: f64,
}

/// The octaves of fractal noise for the use `salt`, each turned by its own random angle.
pub(crate) fn octaves<const N: usize>(seed: u64, salt: u64) -> [Octave; N] {
    std::array::from_fn(|n| {
        let key = key(seed, salt + n as u64);
        let (sin, cos) = (unit(key) * std::f64::consts::TAU).sin_cos();
        Octave { key, cos, sin }
    })
}

/// Fractal noise from 0 to 1: one octave of value noise per octave given, from `period` µm
/// down, each half the last in period and in weight.
///
/// An octave whose period is under two `footprint`s fades to its mean, and is gone at one, so a
/// coarse sampling sees the average rather than aliasing.
pub(crate) fn fractal(octaves: &[Octave], at: Vec2Um, period: f64, footprint: f64) -> f64 {
    let (mut sum, mut total, mut weight, mut period) = (0.0, 0.0, 1.0, period);
    let [x, y] = at.to_um();
    for &Octave { key, cos, sin } in octaves {
        let fade = (period / footprint.max(f64::MIN_POSITIVE) - 1.0).clamp(0.0, 1.0);
        let noise = if fade > 0.0 {
            value(
                key,
                Vec2Um::from_um(x * cos - y * sin, x * sin + y * cos),
                period,
            )
        } else {
            0.5
        };
        sum += weight * mix_values(0.5, noise, fade);
        total += weight;
        weight *= 0.5;
        period *= 0.5;
    }
    sum / total
}

/// How much of a pixel `footprint` wide lies inside an edge at signed `distance` (negative
/// inside): 1 well inside, 0 well outside, and a linear ramp across one footprint.
pub(crate) fn coverage(distance: f64, footprint: f64) -> f64 {
    (0.5 - distance / footprint.max(f64::MIN_POSITIVE)).clamp(0.0, 1.0)
}
