//! Autofocus on the simulated instrument of `config/sim.toml` (ADR-0054): at places on a relief
//! whose height is known, from out of focus, each sweep's focus curve printed, the focus found
//! against the truth, and a thin-plate focus surface through the places found.
//!
//! ```text
//! cargo run --release -p siq-workflow --example autofocus_sim
//! ```
//!
//! The camera is 512 × 384 pixels on a height field of ±20 µm over millimetres, which a frame sees
//! as a tilt of a few micrometres; the stage is the GRBL driver against its emulator, and the
//! focus drive the simulator's instant one. Smoke mode (`SIQ_SMOKE=1`) focuses at one place with
//! a 256 × 192 camera and the `fast` profile.

use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use siq_acquire::{Acquisition, AcquisitionContext};
use siq_buffer::{Backend, PoolConfig};
use siq_calib::{CalibrationRecord, CalibrationRegistry, Fit};
use siq_config::model::{CameraConfig, SimProfile, SimStageConfig};
use siq_devices::{Approach, MoveOpts, Stage, Travel, Xyz};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, duplex};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::peripherals::{SimChanger, SimFocus, SimIllumination};
use siq_sim::state::TrueState;
use siq_synth::{HeightField, HeightFieldParams, World};
use siq_types::{Affine2, Clock, Micrometers, ObjectiveId, SystemClock, Vec2Um};
use siq_workflow::DeviceSet;
use siq_workflow::autofocus::{AutofocusPlan, SurfaceKind, autofocus, depth_of_field, fit_surface};

/// `curve`'s points, a bar each for its sharpness against the sweep's peak.
fn print_curve(name: &str, curve: &[[f64; 2]]) {
    let peak = curve.iter().map(|p| p[1]).fold(0.0, f64::max);
    println!("  {name}:");
    for [z, m] in curve {
        let bar = "#".repeat((40.0 * m / peak).round() as usize);
        println!("    {z:8.2} µm  {m:10.1}  {bar}");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path)?;
    let CameraConfig::Sim(camera) = &mut config.camera else {
        return Err("config/sim.toml has a simulated camera".into());
    };
    let centre = [-20_000.0, -15_000.0];
    let places: Vec<[f64; 2]> = match smoke {
        true => {
            (camera.width, camera.height) = (256, 192);
            config.sim.profile = SimProfile::Fast;
            vec![centre]
        }
        false => {
            (camera.width, camera.height) = (512, 384);
            let d = 150.0;
            [[0.0, 0.0], [d, 0.0], [0.0, d], [-d, 0.0], [0.0, -d]]
                .map(|[dx, dy]| [centre[0] + dx, centre[1] + dy])
                .to_vec()
        }
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let peripherals = &config.peripherals;
    let sources = peripherals
        .illumination
        .as_ref()
        .map_or_else(Vec::new, |l| l.sources.clone());
    let filters = peripherals
        .filter_wheel
        .as_ref()
        .map_or_else(Vec::new, |w| w.filters.clone());
    let state = Arc::new(TrueState::new(sources.len()));
    let relief = Arc::new(HeightField::new(HeightFieldParams {
        amplitude: Micrometers(20.0),
        period: Micrometers(4000.0),
        ..HeightFieldParams::default()
    }));
    let setup = SimCameraSetup::from_config(camera, &config.objectives, &config.sim);
    let (width, height) = setup.sensor;
    let l = setup
        .mount
        .compose(&Affine2::scale(10.0 / 3.76, 10.0 / 3.76));
    let camera = Arc::new(SimCamera::new(
        setup,
        relief.clone(),
        state.clone(),
        clock.clone(),
    ));
    let errors = GrblErrors::from(&config.sim.errors);
    let emulator = GrblEmulator::new(
        GrblSettings::default(),
        errors,
        1,
        state.clone(),
        clock.clone(),
    );
    let (host, device) = duplex(Framing::default(), "grbl");
    tokio::spawn(emulator.run(device));
    let options = GrblOptions::from(&SimStageConfig::default());
    let stage: Arc<dyn Stage> = Arc::new(GrblStage::connect(host, options, clock.clone()).await?);
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots: 8,
        slot_bytes: width as usize * height as usize * 2,
        backend: Backend::Heap,
    };
    let acquisition = Acquisition::new(camera.clone(), &pool, clock.clone())?;
    acquisition.follow_stage(stage.status());
    let settings = tempfile::tempdir()?;
    let calibrations = Arc::new(CalibrationRegistry::open(settings.path())?);
    let fit = Fit {
        affine: l,
        residuals: Vec::new(),
    };
    let record = CalibrationRecord::new(ObjectiveId(1), &fit, None, "the simulator's mount");
    calibrations.save(&record, &[])?;
    acquisition.set_context(AcquisitionContext {
        objective: Some(ObjectiveId(1)),
        calibration: Some(record.id),
        ..acquisition.context()
    });
    let travel = Travel {
        min: Micrometers(-500.0),
        max: Micrometers(500.0),
    };
    let devices = DeviceSet {
        acquisition,
        camera,
        stage,
        focus: Arc::new(SimFocus::new(travel, state.clone(), clock.clone())),
        illumination: Some(Arc::new(SimIllumination::new(
            sources,
            state.clone(),
            clock.clone(),
        ))),
        filter_wheel: Some(Arc::new(SimChanger::filter_wheel(
            filters,
            state.clone(),
            clock.clone(),
        ))),
        turret: None,
        modality: None,
        channels: config.channels.clone(),
        calibrations,
        clock,
    };
    let brightfield = devices.channels[0].clone();
    siq_workflow::routines::channel::apply(&devices, &brightfield).await?;

    // In focus where the focal plane meets the relief: the focus cancels its height and the
    // stage's, where the stage truly is.
    let in_focus = || {
        let pose = state.pose();
        let height = relief.surface_z(Vec2Um::new(pose.x, pose.y));
        -height.map_or(0.0, |h| h.0) - pose.z_coarse.0
    };
    let quarter = depth_of_field(0.30, 1.0, 0.55, 0.376) / 4.0;
    let opts = MoveOpts {
        speed: None,
        approach: Approach::Unidirectional,
    };
    let plan = AutofocusPlan::default();
    let mut found = Vec::new();
    for (k, &[x, y]) in places.iter().enumerate() {
        let at = Xyz {
            x: Micrometers(x),
            y: Micrometers(y),
            z: devices.stage.status().borrow().position.z,
        };
        devices.stage.move_to(at, opts).await?;
        // Out of focus, by up to 20 µm either way.
        let truth = in_focus();
        let from = truth + [15.0, -20.0, 8.0, -12.0, 18.0][k % 5];
        devices.focus.move_to(Micrometers(from), opts).await?;
        let began = Instant::now();
        let focused = autofocus(&devices, &plan).await?;
        let took = began.elapsed().as_secs_f64();
        println!("at ({x}, {y}) µm, from {from:.2} µm, in {took:.1} s:");
        print_curve("coarse, binned", &focused.curve[..plan.coarse as usize]);
        print_curve("fine", &focused.curve[plan.coarse as usize..]);
        let z = focused.z.ok_or("nothing to focus on")?;
        println!(
            "  focus {z:.2} µm, the truth {truth:.2}: {:+.2} µm off, a quarter of the depth of \
             field {quarter:.2}; the sharpness rose by {:.0} %",
            z - truth,
            focused.rise * 100.0
        );
        found.push([x, y, z]);
    }
    if found.len() >= 3 {
        let fitted = fit_surface(&found, SurfaceKind::ThinPlate)?;
        println!("a thin-plate surface through them; each place's focus from the others:");
        for (place, residual) in found.iter().zip(&fitted.residuals) {
            println!("  ({}, {}) µm: {residual:+.2} µm", place[0], place[1]);
        }
    }
    Ok(())
}
