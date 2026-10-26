//! Tile scans (ADR-0053): the planner's coverage, overlap, clipping and orders as properties; and a
//! 3 × 3 scan on the simulated instrument, through the GRBL driver against its emulator, giving
//! the dataset expected and taking about the time estimated. REQ-WF-001, REQ-STG-005, REQ-STI-003.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use proptest::prelude::*;
use siq_acquire::{Acquisition, AcquisitionContext};
use siq_buffer::{Backend, PoolConfig};
use siq_calib::{CalibrationRecord, CalibrationRegistry, Fit};
use siq_config::model::{CameraConfig, SimStageConfig};
use siq_devices::mock::MockFocusDrive;
use siq_devices::{Stage, Travel};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, duplex};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::peripherals::{SimChanger, SimIllumination};
use siq_sim::state::TrueState;
use siq_storage::{Dataset, DatasetKind, DatasetReader, Snapshot};
use siq_synth::WorldInfo;
use siq_types::{Affine2, AffineParams, ChannelId, Clock, Micrometers, ObjectiveId, SystemClock};
use siq_workflow::DeviceSet;
use siq_workflow::engine::{
    DatasetTarget, ErrorPolicy, LAYOUT, Outcome, Plan, RunControl, Step, run,
};
use siq_workflow::scan::{Grid, Order, Region, Timing, estimate, plan};

const FIELD: [f64; 2] = [256.0, 192.0];

/// A camera at 0.376 µm a pixel, turned `rotation` degrees.
fn camera(rotation: f64) -> Affine2 {
    Affine2::from_params(&AffineParams {
        scale_x: 1.0 / 0.376,
        scale_y: 1.0 / 0.376,
        rotation: rotation.to_radians(),
        shear: 0.001,
        translation: [0.0; 2],
    })
}

/// Whether stage point `at` is in the frame of `tile`.
fn in_frame(l: &Affine2, tile: [f64; 2], at: [f64; 2]) -> bool {
    let [x, y] = l.apply_vector([at[0] - tile[0], at[1] - tile[1]]);
    x.abs() <= FIELD[0] / 2.0 + 1e-6 && y.abs() <= FIELD[1] / 2.0 + 1e-6
}

/// Whether every place of `places` is in some tile's frame.
fn covered(l: &Affine2, grid: &Grid, places: impl IntoIterator<Item = [f64; 2]>) -> bool {
    let mut places = places.into_iter();
    places.all(|at| grid.tiles.iter().any(|t| in_frame(l, [t.x, t.y], at)))
}

/// A 9 × 9 lattice over the rectangle, its edges included.
fn lattice([x0, y0, x1, y1]: [f64; 4]) -> impl Iterator<Item = [f64; 2]> {
    (0..=8).flat_map(move |j| {
        (0..=8).map(move |i| {
            let (u, v) = (f64::from(i) / 8.0, f64::from(j) / 8.0);
            [x0 + u * (x1 - x0), y0 + v * (y1 - y0)]
        })
    })
}

proptest! {
    #[test]
    fn a_rectangle_is_covered_with_the_overlap_asked(
        (x0, y0) in (-5000.0..0.0f64, -5000.0..0.0f64),
        (width, height) in (20.0..600.0f64, 20.0..500.0f64),
        rotation in -1.0..1.0f64,
        overlap in 0.0..0.5f64,
        order in prop_oneof![Just(Order::Snake), Just(Order::Raster), Just(Order::Nearest)],
    ) {
        let l = camera(rotation);
        let (x1, y1) = (x0 + width, y0 + height);
        let region = Region::Rect { x0, y0, x1, y1 };
        let grid = plan(&region, &l, FIELD, overlap, order).unwrap();
        prop_assert!(covered(&l, &grid, lattice([x0, y0, x1, y1])));
        // Neighbours are a step apart, which moves the image by (1 − overlap) of the frame.
        let [along, down] = grid.steps.map(|step| l.apply_vector(step));
        prop_assert!((along[0] - (1.0 - overlap) * FIELD[0]).abs() < 1e-6 && along[1].abs() < 1e-6);
        prop_assert!((down[1] - (1.0 - overlap) * FIELD[1]).abs() < 1e-6 && down[0].abs() < 1e-6);
        for a in &grid.tiles {
            for b in &grid.tiles {
                if (a.row, a.column + 1) == (b.row, b.column) {
                    prop_assert!((b.x - a.x - grid.steps[0][0]).abs() < 1e-6);
                    prop_assert!((b.y - a.y - grid.steps[0][1]).abs() < 1e-6);
                }
            }
        }
        // Each tile once.
        let mut seen: Vec<_> = grid.tiles.iter().map(|t| (t.row, t.column)).collect();
        seen.sort();
        seen.dedup();
        prop_assert_eq!(seen.len(), grid.tiles.len());
    }
}

#[test]
fn a_polygon_keeps_only_the_tiles_that_meet_it() {
    // An L: a 600 × 600 µm square without its top-right 400 × 400.
    let l = camera(0.0);
    let points = vec![
        [0.0, 0.0],
        [600.0, 0.0],
        [600.0, 200.0],
        [200.0, 200.0],
        [200.0, 600.0],
        [0.0, 600.0],
    ];
    let polygon = Region::Polygon {
        points: points.clone(),
    };
    let square = Region::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 600.0,
        y1: 600.0,
    };
    let clipped = plan(&polygon, &l, FIELD, 0.1, Order::Snake).unwrap();
    let whole = plan(&square, &l, FIELD, 0.1, Order::Snake).unwrap();
    assert!(
        clipped.tiles.len() < whole.tiles.len(),
        "{} of {}",
        clipped.tiles.len(),
        whole.tiles.len()
    );
    let inside = |[x, y]: [f64; 2]| x <= 200.0 || y <= 200.0;
    let places = lattice([0.0, 0.0, 600.0, 600.0]).filter(|at| inside(*at));
    assert!(covered(&l, &clipped, places));
    // A tile whose frame lies wholly in the missing corner is left out.
    let corner = |t: &siq_workflow::scan::Tile| {
        let half = [FIELD[0] * 0.376 / 2.0, FIELD[1] * 0.376 / 2.0];
        t.x - half[0] > 200.0 && t.y - half[1] > 200.0
    };
    assert!(whole.tiles.iter().any(corner));
    assert!(!clipped.tiles.iter().any(corner));
    assert!(
        plan(
            &Region::Polygon {
                points: points[..2].to_vec()
            },
            &l,
            FIELD,
            0.1,
            Order::Snake
        )
        .is_err()
    );
    assert!(plan(&square, &l, FIELD, 0.95, Order::Snake).is_err());
}

#[test]
fn the_orders_visit_every_tile_once_as_named() {
    let l = camera(0.0);
    let region = Region::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 300.0,
        y1: 200.0,
    };
    let path = |order| {
        let grid = plan(&region, &l, FIELD, 0.1, order).unwrap();
        grid.tiles
            .iter()
            .map(|t| (t.row, t.column))
            .collect::<Vec<_>>()
    };
    let snake = path(Order::Snake);
    assert_eq!(
        &snake[..6],
        [(0, 0), (0, 1), (0, 2), (0, 3), (1, 3), (1, 2)]
    );
    let raster = path(Order::Raster);
    assert_eq!(
        &raster[..6],
        [(0, 0), (0, 1), (0, 2), (0, 3), (1, 0), (1, 1)]
    );
    let mut nearest = path(Order::Nearest);
    assert_eq!(nearest[0], (0, 0));
    nearest.sort();
    let mut every = raster.clone();
    every.sort();
    assert_eq!(nearest, every);
}

/// The simulated instrument at 256 × 192, the stage through the GRBL driver against its emulator,
/// calibrated with the simulator's own mount.
async fn instrument() -> (DeviceSet, DatasetTarget, [tempfile::TempDir; 2]) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path).unwrap();
    let CameraConfig::Sim(camera) = &mut config.camera else {
        panic!("a simulated camera")
    };
    (camera.width, camera.height) = (256, 192);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let peripherals = &config.peripherals;
    let sources = peripherals.illumination.as_ref().unwrap().sources.clone();
    let filters = peripherals.filter_wheel.as_ref().unwrap().filters.clone();
    let state = Arc::new(TrueState::new(sources.len()));
    let world = WorldInfo::from_config(&config.sim.world, config.sim.seed)
        .build()
        .unwrap();
    let setup = SimCameraSetup::from_config(camera, &config.objectives, &config.sim);
    let l = setup
        .mount
        .compose(&Affine2::scale(10.0 / 3.76, 10.0 / 3.76));
    let camera = Arc::new(SimCamera::new(
        setup,
        world.into(),
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
    let stage: Arc<dyn Stage> = Arc::new(
        GrblStage::connect(host, options, clock.clone())
            .await
            .unwrap(),
    );
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots: 8,
        slot_bytes: 256 * 192 * 2,
        backend: Backend::Heap,
    };
    let acquisition = Acquisition::new(camera.clone(), &pool, clock.clone()).unwrap();
    acquisition.follow_stage(stage.status());
    let folders = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let calibrations = Arc::new(CalibrationRegistry::open(folders[0].path()).unwrap());
    let fit = Fit {
        affine: l,
        residuals: Vec::new(),
    };
    let record = CalibrationRecord::new(ObjectiveId(1), &fit, None, "the simulator's mount");
    calibrations.save(&record, &[]).unwrap();
    acquisition.set_context(AcquisitionContext {
        objective: Some(ObjectiveId(1)),
        calibration: Some(record.id),
        ..acquisition.context()
    });
    let focus_travel = Travel {
        min: Micrometers(0.0),
        max: Micrometers(10_000.0),
    };
    let devices = DeviceSet {
        acquisition,
        camera,
        stage,
        focus: Arc::new(MockFocusDrive::new(focus_travel, clock.clone())),
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
    let target = DatasetTarget {
        data_root: folders[1].path().to_owned(),
        min_free: 0,
        catalog: None,
        instrument: Some(Snapshot::of(&config).unwrap()),
    };
    (devices, target, folders)
}

#[tokio::test]
async fn a_3_by_3_scan_on_the_simulator_gives_its_dataset_in_about_the_time_estimated() {
    let (devices, target, _folders) = instrument().await;
    // 2.6 steps of 0.9 of a frame each way: three tiles across, three down.
    let (centre, l) = (
        [-20_000.0, -15_000.0],
        devices.calibrations.history(ObjectiveId(1))[0].affine(),
    );
    let step = [0.9 * 256.0 * 0.376 / 1.002, 0.9 * 192.0 * 0.376 / 1.002];
    let (hw, hh) = (1.3 * step[0], 1.3 * step[1]);
    let region = Region::Rect {
        x0: centre[0] - hw,
        y0: centre[1] - hh,
        x1: centre[0] + hw,
        y1: centre[1] + hh,
    };
    let scan = Step::TileScan {
        region: region.clone(),
        overlap: 0.1,
        order: Order::Snake,
        channels: vec![ChannelId(1)],
        settle_ms: 20.0,
        focus: None,
    };
    let plan_ = Plan {
        name: "tile-scan".to_owned(),
        steps: vec![scan],
        on_error: ErrorPolicy::default(),
    };
    let grid = plan(&region, &l, FIELD, 0.1, Order::Snake).unwrap();
    assert_eq!(grid.tiles.len(), 9);
    let exposure = devices.channels[0].exposure.0 / 1000.0;
    let at = devices.stage.status().borrow().position;
    let expected = estimate(
        &grid,
        [at.x.0, at.y.0],
        0.02,
        &[exposure],
        256 * 192 * 2,
        &Timing::default(),
    );

    let began = Instant::now();
    let report = run(&devices, &target, &plan_, &RunControl::new(), &|_| {})
        .await
        .unwrap();
    let took = began.elapsed().as_secs_f64();
    assert_eq!(
        (report.outcome, report.frames),
        (Outcome::Finished, 9),
        "{report:?}"
    );
    let ratio = took / expected.seconds;
    assert!(expected.fits(u64::MAX, 0) && !expected.fits(expected.bytes, 1));
    assert!(
        (0.8..1.2).contains(&ratio),
        "took {took:.2} s, estimated {:.2} s",
        expected.seconds
    );

    // The dataset: a tile scan, its layout, and a frame at each tile, where the tile is.
    let dataset = Dataset::open(&report.path).unwrap();
    assert_eq!(dataset.manifest().kind, DatasetKind::TileScan);
    let layout: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(report.path.join(LAYOUT)).unwrap()).unwrap();
    let tiles = layout["scans"][0]["tiles"].as_array().unwrap();
    assert_eq!(tiles.len(), 9);
    let reader = DatasetReader::open(&dataset).unwrap();
    let mut frames: Vec<_> = reader.frames().iter().map(|frame| frame.sidecar).collect();
    frames.sort_by_key(|frame| frame.key.p);
    for (frame, tile) in frames.iter().zip(&grid.tiles) {
        assert_eq!(frame.key.c, 1);
        let pose = frame.meta.pose.unwrap();
        let off = (pose.x.0 - tile.x).hypot(pose.y.0 - tile.y);
        assert!(
            off < 1.0,
            "p{} at ({}, {}), planned ({}, {})",
            frame.key.p,
            pose.x.0,
            pose.y.0,
            tile.x,
            tile.y
        );
    }
    assert_eq!(
        frames.iter().map(|f| f.key.p).collect::<Vec<_>>(),
        (0..9).collect::<Vec<_>>()
    );
}
