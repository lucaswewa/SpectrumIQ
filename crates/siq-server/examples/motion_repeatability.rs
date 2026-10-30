//! The stage's repeatability and backlash (P10.3), as the calibration measures them but over more
//! repeats: one place approached from −x −y and from +x +y, `--repeats` times, a frame at each
//! arrival registered against the first from the same side, and against the arrival from the other
//! side.
//!
//! ```text
//! cargo run --release -p siq-server --example motion_repeatability [-- -c <config>] [--repeats N] [--reach µm]
//! ```
//!
//! Without `-c`, the simulated instrument of `config/sim.toml`, whose stage's noise and backlash are
//! set against what is found. With one, the instrument it describes: something textured must be in
//! the field, and the stage free to move `--reach` µm (50 by default) each way. Shifts are turned
//! into µm by the calibration in force for the objective, or by the sensor's pitch through the
//! objective without one. 10 repeats by default; smoke mode (`SIQ_SMOKE=1`) takes 3, at 512 × 384.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use siq_calib::{Fit, backlash, repeatability};
use siq_config::model::CameraConfig;
use siq_register::Registrar;
use siq_server::Service;
use siq_types::Affine2;
use siq_workflow::routines::stage_camera::{BacklashPlan, StageCameraPlan, measure};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let args: Vec<String> = std::env::args().collect();
    let after = |flag: &str| {
        let at = args.iter().position(|arg| arg == flag)?;
        args.get(at + 1).cloned()
    };
    let temporary = tempfile::tempdir()?;
    let simulated = after("-c").is_none();
    let path = after("-c").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml"),
        PathBuf::from,
    );
    let mut config = siq_config::load(&path)?;
    let repeats =
        after("--repeats").map_or(if smoke { 3 } else { 10 }, |n| n.parse().unwrap_or(10));
    let reach = after("--reach").map_or(50.0, |reach| reach.parse().unwrap_or(50.0));
    if simulated {
        config.server.settings = temporary.path().to_owned();
        if let CameraConfig::Sim(camera) = &mut config.camera {
            (camera.width, camera.height) = if smoke { (512, 384) } else { (2048, 1536) };
        }
    }
    config.server.audit = temporary.path().join("audit");
    let errors = config.sim.errors.clone();
    let (_, instrument) = Service::build(config).await?.into_parts();
    if simulated {
        let somewhere = siq_devices::Xyz::from_um(-20_000.0, -15_000.0, 0.0);
        instrument
            .stage
            .move_to(somewhere, Default::default())
            .await?;
    }

    // Stage µm to sensor pixels: the calibration in force, or the pitch through the objective.
    let context = instrument.acquisition.context();
    let calibration = context
        .calibration
        .and_then(|id| instrument.calibrations.get(id));
    let nominal = || {
        let objective = instrument
            .config
            .objectives
            .iter()
            .find(|objective| Some(objective.id) == context.objective);
        let magnification = objective.map_or(1.0, |objective| objective.magnification);
        let scale = magnification / instrument.camera.capabilities().pixel_size.0;
        Affine2::scale(scale, scale)
    };
    let affine = calibration
        .as_ref()
        .map_or_else(nominal, |record| record.affine());
    let fit = Fit {
        affine,
        residuals: Vec::new(),
    };
    println!(
        "{} repeats from {reach} µm each side, by {}",
        repeats,
        match calibration {
            Some(record) => format!("calibration {}", record.id),
            None => "the sensor's pitch through the objective".to_owned(),
        }
    );

    let plan = StageCameraPlan {
        moves: Vec::new(),
        backlash: Some(BacklashPlan {
            at: [0.0, 0.0],
            reach,
            repeats,
        }),
        min_pce: 100.0,
    };
    let began = Instant::now();
    let registrar = Arc::new(Registrar::default());
    let stage = instrument.stage.as_ref();
    let measured = measure(&instrument.acquisition, stage, &registrar, &plan).await?;
    println!("measured in {:.1?}", began.elapsed());
    let inverse = fit.affine.invert().ok_or("the calibration is singular")?;
    for (k, shift) in measured.repeats.iter().enumerate() {
        let [x, y] = inverse.apply_vector(*shift);
        println!("arrival {}: {x:+.3}, {y:+.3} µm from the first", k + 2);
    }
    if let Some([x, y]) = repeatability(&fit, &measured.repeats) {
        print!("repeatability {x:.3} µm in x, {y:.3} in y, a standard deviation");
        match simulated {
            true => println!(" ({} µm of noise injected)", errors.position_noise.0),
            false => println!(),
        }
    }
    if let Some([x, y]) = measured.backlash_shift.map(|shift| backlash(&fit, shift)) {
        print!("backlash {x:.3} µm in x, {y:.3} in y");
        match simulated {
            true => println!(" ({} injected)", errors.backlash.0),
            false => println!(),
        }
    }
    Ok(())
}
