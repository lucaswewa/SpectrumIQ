//! Digitization: shot noise, read noise, the offsets, gain and quantization.
//!
//! Every random number is a hash of the sensor pixel, and of the frame for the temporal noise,
//! so a frame repeats exactly for its seed and number, and an ROI is exactly a crop.

use std::sync::OnceLock;

use siq_imgproc::{KernelError, for_each_row};
use siq_types::{ImageView, ImageViewMut};

use super::{OpticsModel, SensorModel, Shot};

/// SplitMix64's finaliser.
const fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// The key of one use of randomness: a seed, and what it is for.
pub(crate) const fn key(seed: u64, purpose: u64) -> u64 {
    mix(seed ^ mix(purpose))
}

/// The hash of sensor pixel `(x, y)` under `key`.
pub(crate) const fn hash(key: u64, x: u64, y: u64) -> u64 {
    mix(key
        .wrapping_add(x.wrapping_mul(0x9e37_79b9_7f4a_7c15))
        .wrapping_add(y.wrapping_mul(0xc2b2_ae3d_27d4_eb4f)))
}

/// What the fixed pattern's keys are for.
pub(crate) const PRNU: u64 = 1;
const DSNU: u64 = 2;

/// A standard normal deviate from the top 16 bits of `hash`: one of 64 Ki quantiles.
pub(crate) fn gaussian(hash: u64) -> f32 {
    static TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..1 << 16)
            .map(|i| quantile((f64::from(i) + 0.5) / 65536.0) as f32)
            .collect()
    });
    table[(hash >> 48) as usize]
}

/// The standard normal quantile of `p`, by Acklam's rational approximation (relative error under
/// 1.2·10⁻⁹).
fn quantile(p: f64) -> f64 {
    const A: [f64; 6] = [
        -39.696_830_286_653_76,
        220.946_098_424_520_5,
        -275.928_510_446_968_7,
        138.357_751_867_269,
        -30.664_798_066_147_16,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -54.476_098_798_224_06,
        161.585_836_858_040_9,
        -155.698_979_859_886_6,
        66.801_311_887_719_72,
        -13.280_681_552_885_72,
    ];
    const C: [f64; 6] = [
        -0.007_784_894_002_430_293,
        -0.322_396_458_041_136_4,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        0.007_784_695_709_041_462,
        0.322_467_129_070_039_8,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    let tail = |q: f64| {
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    match p {
        _ if p < 0.024_25 => tail((-2.0 * p.ln()).sqrt()),
        _ if p > 1.0 - 0.024_25 => -tail((-2.0 * (1.0 - p).ln()).sqrt()),
        _ => {
            let (q, r) = (p - 0.5, (p - 0.5).powi(2));
            (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
                / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
        }
    }
}

/// A Poisson count of mean `mean`, by inversion from `hash`: for the small means where the
/// Gaussian approximation fails.
fn poisson(mean: f64, hash: u64) -> f64 {
    let u = (hash >> 11) as f64 / (1u64 << 53) as f64;
    let (mut k, mut p) = (0.0, (-mean).exp());
    let mut total = p;
    while u > total && k < 200.0 {
        k += 1.0;
        p *= mean / k;
        total += p;
    }
    k
}

/// Above this many electrons, shot noise is Gaussian.
const GAUSSIAN_ABOVE: f64 = 20.0;

/// Digitizes `signal`, as [`render_signal`](super::render_signal) leaves it, into frame `seq`
/// of `shot`:
///
/// - electrons are the signal times the flux and the exposure, and on a colour sensor the
///   response of the pixel's colour: binned, a pixel keeps its place in the pattern;
/// - shot noise, Poisson below 20 e⁻ and Gaussian above, and the read noise of every binned
///   pixel;
/// - divided by the electrons per ADU, times the gain;
/// - plus the dark offset and the pixel's own offset;
/// - rounded, and clipped to the pixel format.
pub fn digitize(
    signal: &ImageView<'_, f32>,
    optics: &OpticsModel,
    sensor: &SensorModel,
    shot: &Shot,
    seq: u64,
    out: &mut ImageViewMut<'_, u16>,
) -> Result<(), KernelError> {
    let expected = shot.image_size();
    for (what, found) in [
        ("the signal", (signal.width(), signal.height())),
        ("the frame", (out.width(), out.height())),
    ] {
        if found != expected {
            return Err(KernelError::Size {
                what,
                expected,
                found,
            });
        }
    }
    let settings = &shot.settings;
    let (bx, by) = settings.binning.factors();
    let (bx, by) = (u64::from(bx), u64::from(by));
    let electrons = optics.flux * settings.exposure.0;
    let adu = settings.gain / sensor.electrons_per_adu;
    let read_variance = sensor.read_noise.powi(2) * (bx * by) as f64;
    let max = f64::from(settings.pixel_format.max_value());
    let (frame, fixed) = (key(sensor.seed, 1000 + seq), key(sensor.seed, DSNU));
    let (x0, y0) = (
        u64::from(settings.roi.offset_x),
        u64::from(settings.roi.offset_y),
    );
    let roi = settings.roi;
    let cfa = sensor
        .cfa
        .map(|cfa| cfa.shifted(roi.offset_x, roi.offset_y));
    for_each_row(out, |row, line| {
        let sy = y0 + row as u64 * by;
        for ((column, value), &s) in (0u64..).zip(line.iter_mut()).zip(signal.row(row)) {
            let sx = x0 + column * bx;
            let response = cfa.map_or(1.0, |cfa| sensor.response[cfa.colour(column as usize, row)]);
            let (mean, h) = (f64::from(s) * electrons * response, hash(frame, sx, sy));
            let collected = match mean > GAUSSIAN_ABOVE {
                true => mean + (mean + read_variance).sqrt() * f64::from(gaussian(h)),
                false => {
                    poisson(mean.max(0.0), h) + read_variance.sqrt() * f64::from(gaussian(mix(h)))
                }
            };
            let offset =
                sensor.dark_offset + sensor.dsnu * f64::from(gaussian(hash(fixed, sx, sy)));
            *value = (collected * adu + offset).round().clamp(0.0, max) as u16;
        }
    });
    Ok(())
}
