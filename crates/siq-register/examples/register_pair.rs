//! Pairs of frames from the simulated camera, the stage moved between them by a known amount,
//! registered: the shift found against the move, its confidence, and the time it took.
//!
//! ```text
//! cargo run --release -p siq-register --example register_pair
//! ```
//!
//! The frames are 2048 × 1536 of the cells world at 0.376 µm a pixel, with the camera's noise and
//! vignetting, and a pair of 4096 × 3072 coarse to fine. Moving the stage by `m` moves the content
//! by `−m` over the pixel size. Smoke mode (`SIQ_SMOKE=1`) uses 512 × 384.

use std::time::Instant;

use siq_devices::{CameraSettings, TriggerMode};
use siq_register::Registrar;
use siq_sim::optics::{OpticsModel, Scratch, SensorModel, Shot, form_image};
use siq_synth::{Cells, CellsParams};
use siq_types::{Affine2, Binning, Micrometers, Millis, OwnedImage, PixelFormat, Roi, StagePose};

/// The pixel size on the sample: a 3.76 µm sensor behind a 10× objective.
const PIXEL: f64 = 0.376;

/// A frame of `size` with the stage at (`x`, `y`) µm.
fn frame(world: &Cells, size: (u32, u32), (x, y): (f64, f64), seq: u64) -> OwnedImage<u16> {
    let shot = Shot {
        pose: StagePose {
            x: Micrometers(x),
            y: Micrometers(y),
            ..StagePose::default()
        },
        calibration: Affine2::scale(1.0 / PIXEL, 1.0 / PIXEL),
        sensor: size,
        settings: CameraSettings {
            roi: Roi::full(size.0, size.1),
            binning: Binning::NONE,
            pixel_format: PixelFormat::Mono14,
            exposure: Millis(10.0),
            gain: 1.0,
            trigger: TriggerMode::Software,
        },
    };
    let mut out = OwnedImage::new(size.0 as usize, size.1 as usize);
    let (optics, sensor) = (OpticsModel::default(), SensorModel::default());
    form_image(
        world,
        &optics,
        &sensor,
        &shot,
        seq,
        &mut Scratch::default(),
        &mut out.view_mut(),
    )
    .expect("the frame fits its shot");
    out
}

fn main() {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let world = Cells::new(CellsParams {
        seed: 1,
        ..CellsParams::default()
    });
    let registrar = Registrar::default();
    let base = if smoke { (512, 384) } else { (2048, 1536) };
    let third = f64::from(base.0) * PIXEL / 3.0;
    let mut pairs = vec![
        (base, (1.13, -0.77)),
        (base, (15.2, 9.71)),
        (base, (-94.0, 49.1)),
        (base, (third, -third / 2.0)),
    ];
    if !smoke {
        pairs.push(((4096, 3072), (-401.3, 211.9)));
    }
    println!(
        "frames        move (µm)            expected (px)      found (px)         error   PCE      margin  NCC    ms"
    );
    for (k, &(size, (mx, my))) in pairs.iter().enumerate() {
        let a = frame(&world, size, (0.0, 0.0), 2 * k as u64);
        let b = frame(&world, size, (mx, my), 2 * k as u64 + 1);
        let started = Instant::now();
        let found = registrar
            .register(a.view(), b.view())
            .expect("the frames register");
        let ms = started.elapsed().as_secs_f64() * 1e3;
        let expected = [-mx / PIXEL, -my / PIXEL];
        let error = (found.shift[0] - expected[0]).hypot(found.shift[1] - expected[1]);
        println!(
            "{:>4} × {:<4}  ({mx:>7.2}, {my:>7.2})  ({:>8.2}, {:>8.2})  ({:>8.2}, {:>8.2})  {error:.3}  {:>7.0}  {:.2}    {:.3}  {ms:.1}",
            size.0,
            size.1,
            expected[0],
            expected[1],
            found.shift[0],
            found.shift[1],
            found.pce,
            found.margin,
            found.ncc
        );
    }
}
