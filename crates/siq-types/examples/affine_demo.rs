//! Builds a stage↔pixel calibration and maps a few points with it (ADR-0018).
//!
//! ```text
//! cargo run -p siq-types --example affine_demo
//! ```
//!
//! It also checks its own results, and exits with an error if a mapping is wrong. Smoke mode
//! (`SIQ_SMOKE=1`) runs the same, quick, demonstration.

use std::process::ExitCode;

use siq_types::{Affine2, AffineParams, Binning, PixelToStage, PointPx, Roi, StageToPixel, Vec2Um};

fn main() -> ExitCode {
    // 3.45 µm pixels behind a 10× objective: 0.345 µm per pixel, so 2.9 px per µm. The camera is
    // turned 0.5° on the microscope, and its rows run down while the stage's y runs up: a
    // negative scale_y.
    let px_per_um = 1.0 / 0.345;
    let calibration = Affine2::from_params(&AffineParams {
        scale_x: px_per_um,
        scale_y: -px_per_um,
        rotation: 0.5_f64.to_radians(),
        shear: 0.0,
        translation: [0.0, 0.0],
    });
    println!("calibration (px per µm): {:?}", calibration.rows());
    let params = calibration
        .decompose()
        .expect("a calibration is invertible");
    println!(
        "  decomposed: scale {:.4} × {:.4} px/µm, rotation {:.3}°",
        params.scale_x,
        params.scale_y,
        params.rotation.to_degrees()
    );

    // A 11648 × 8742 sensor, its centre on the optical axis.
    let (width, height) = (11_648, 8_742);
    let centre = PointPx::new(f64::from(width) / 2.0, f64::from(height) / 2.0);
    let stage = Vec2Um::from_um(25_000.0, 12_000.0);
    let full = StageToPixel::for_frame(
        &calibration,
        stage,
        centre,
        Roi::full(width, height),
        Binning::NONE,
    );

    let mut ok = true;
    let mut show = |label: &str, mapping: &StageToPixel, point: Vec2Um, expected: PointPx| {
        let pixel = mapping.apply(point);
        let error = (pixel - expected).to_array().map(f64::abs);
        let good = error[0] < 1e-6 && error[1] < 1e-6;
        ok &= good;
        println!(
            "  {label:<34} ({:>9.2}, {:>9.2}) µm -> ({:>9.2}, {:>9.2}) px{}",
            point.x.get(),
            point.y.get(),
            pixel.x,
            pixel.y,
            if good { "" } else { "  <- wrong" }
        );
    };

    println!("\nfull frame, stage at ({}, {}):", stage.x, stage.y);
    show("the stage position", &full, stage, centre);
    let right = stage + Vec2Um::from_um(100.0, 0.0);
    let [dx, dy] = calibration.apply_vector([100.0, 0.0]);
    show(
        "100 µm along +x",
        &full,
        right,
        PointPx::new(centre.x + dx, centre.y + dy),
    );

    // The same view read out as a 4096 × 4096 region with 2 × 2 binning.
    let roi = Roi {
        offset_x: 3776,
        offset_y: 2323,
        width: 4096,
        height: 4096,
    };
    let binning = Binning::new(2, 2).expect("non-zero");
    let binned = StageToPixel::for_frame(&calibration, stage, centre, roi, binning);
    println!("\n4096 × 4096 region at (3776, 2323), binned 2 × 2:");
    let roi_centre = PointPx::new((centre.x - 3776.0) / 2.0, (centre.y - 2323.0) / 2.0);
    show("the stage position", &binned, stage, roi_centre);

    // And back: which sample position is under the region's top-left pixel centre?
    let inverse: PixelToStage = binned.inverse().expect("invertible");
    let corner = inverse.apply(PointPx::pixel_centre(0, 0));
    println!(
        "\nthe region's first pixel centre sees the sample at ({:.2}, {:.2}) µm",
        corner.x.get(),
        corner.y.get()
    );
    let round_trip = binned.apply(corner) - PointPx::pixel_centre(0, 0);
    ok &= round_trip.x.abs() < 1e-6 && round_trip.y.abs() < 1e-6;

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
