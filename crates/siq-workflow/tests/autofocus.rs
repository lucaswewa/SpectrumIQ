//! Autofocus and focus surfaces (ADR-0054) on the simulated instrument: focus found within a
//! quarter of the depth of field on a tilted, curved specimen; nothing to focus on, said, on a
//! bare field; the surfaces' fits; and M6's criterion, a 5 × 5 tile scan with a 9-point focus map
//! in focus at every tile. REQ-WF-002.

mod common;

use std::sync::{Arc, Mutex};

use siq_sim::state::TrueState;
use siq_synth::{Grains, GrainsParams, World, WorldInfo};
use siq_types::{Micrometers, Vec2Um};
use siq_workflow::autofocus::{AutofocusPlan, SurfaceKind, autofocus, depth_of_field, fit_surface};
use siq_workflow::engine::{
    ErrorPolicy, LAYOUT, Outcome, Plan, RunControl, RunError, RunEvent, Step, TileFocus, run,
};
use siq_workflow::scan::{Order, Region};

/// The objective's depth of field: 10×/0.30 in green, at 0.376 µm a pixel.
fn dof() -> f64 {
    depth_of_field(0.30, 1.0, 0.55, 0.376)
}

/// Grains, which have texture everywhere, on a surface tilted and curved about `centre`:
/// `tilt·d + curvature·|d|²`, in µm.
struct Specimen {
    grains: Grains,
    centre: [f64; 2],
    tilt: [f64; 2],
    curvature: f64,
}

impl Specimen {
    fn new() -> Self {
        Self {
            grains: Grains::new(GrainsParams::default()),
            centre: [-20_000.0, -15_000.0],
            tilt: [0.03, -0.02],
            curvature: 1e-4,
        }
    }

    fn height(&self, [x, y]: [f64; 2]) -> f64 {
        let (dx, dy) = (x - self.centre[0], y - self.centre[1]);
        self.tilt[0] * dx + self.tilt[1] * dy + self.curvature * (dx * dx + dy * dy)
    }

    /// The focus at which it is sharp at the stage's `(x, y)`, the stage's z `z` µm: the one that
    /// cancels its height and the stage's.
    fn in_focus(&self, at: [f64; 2], z: f64) -> f64 {
        -self.height(at) - z
    }

    /// The focus at which it is sharp where the stage truly is, by the simulator's state.
    fn in_focus_now(&self, state: &TrueState) -> f64 {
        let pose = state.pose();
        self.in_focus([pose.x.0, pose.y.0], pose.z_coarse.0)
    }
}

impl World for Specimen {
    fn intensity(&self, at: Vec2Um, footprint: Micrometers) -> f32 {
        self.grains.intensity(at, footprint)
    }

    fn surface_z(&self, at: Vec2Um) -> Option<Micrometers> {
        Some(Micrometers(self.height(at.to_um())))
    }

    fn info(&self) -> WorldInfo {
        self.grains.info()
    }
}

/// A blank slide: the same light everywhere.
struct Blank;

impl World for Blank {
    fn intensity(&self, _: Vec2Um, _: Micrometers) -> f32 {
        0.8
    }

    fn info(&self) -> WorldInfo {
        unreachable!("not described")
    }
}

async fn go(devices: &siq_workflow::DeviceSet, [x, y]: [f64; 2], focus: f64) {
    let opts = siq_devices::MoveOpts {
        speed: None,
        approach: siq_devices::Approach::Direct,
    };
    let at = siq_devices::Xyz {
        x: Micrometers(x),
        y: Micrometers(y),
        z: Micrometers(0.0),
    };
    devices.stage.move_to(at, opts).await.unwrap();
    devices
        .focus
        .move_to(Micrometers(focus), opts)
        .await
        .unwrap();
}

#[tokio::test]
async fn autofocus_finds_focus_within_a_quarter_of_the_depth_of_field() {
    let specimen = Arc::new(Specimen::new());
    let c = specimen.centre;
    let places = [
        ([c[0], c[1]], 12.0),
        ([c[0] + 150.0, c[1]], -15.0),
        ([c[0], c[1] + 150.0], 8.0),
        ([c[0] - 150.0, c[1] - 120.0], -20.0),
        ([c[0] + 120.0, c[1] - 150.0], 3.0),
    ];
    focuses(specimen, &places, None).await;
}

// ADR-0071: a colour sensor's frames are scored on their luminance, as well.
#[tokio::test]
async fn a_colour_camera_autofocuses_on_luminance() {
    let specimen = Arc::new(Specimen::new());
    let c = specimen.centre;
    let places = [([c[0], c[1]], 12.0), ([c[0] - 150.0, c[1] - 120.0], -20.0)];
    focuses(specimen, &places, Some(siq_types::Cfa::Gbrg)).await;
}

/// Autofocuses at each of `places`, from its offset out of focus, on a sensor with `cfa`.
async fn focuses(specimen: Arc<Specimen>, places: &[([f64; 2], f64)], cfa: Option<siq_types::Cfa>) {
    let world: Arc<dyn World> = specimen.clone();
    let rig = common::instrument_in((256, 192), Some(world), cfa).await;
    let common::Rig { devices, state, .. } = rig;
    for &(at, offset) in places {
        go(&devices, at, specimen.in_focus(at, 0.0) + offset).await;
        let truth = specimen.in_focus_now(&state);
        let found = autofocus(&devices, &AutofocusPlan::default())
            .await
            .unwrap();
        let z = found.z.expect("something to focus on");
        assert!(
            (z - truth).abs() < dof() / 4.0,
            "at {at:?}, from {offset} µm out: {:.2} µm off, a quarter of the depth of field is {:.2}",
            z - truth,
            dof() / 4.0
        );
        assert_eq!(found.curve.len(), 24);
        assert!(found.rise > 0.5, "{}", found.rise);
        assert_eq!(devices.focus.status().borrow().position.0, z);
    }
    // The camera is as it was.
    assert_eq!(devices.camera.settings().binning, siq_types::Binning::NONE);
}

#[tokio::test]
async fn a_bare_field_has_nothing_to_focus_on() {
    let common::Rig {
        devices, target, ..
    } = common::instrument((256, 192), Some(Arc::new(Blank))).await;
    go(&devices, [-20_000.0, -15_000.0], 5.0).await;
    let found = autofocus(&devices, &AutofocusPlan::default())
        .await
        .unwrap();
    assert_eq!(found.z, None, "{found:?}");
    assert!(found.rise < 0.2, "{}", found.rise);
    assert_eq!(
        devices.focus.status().borrow().position.0,
        5.0,
        "back where it was"
    );

    // As a step, it fails, and its policy decides.
    let plan = Plan {
        name: "bare".to_owned(),
        steps: vec![Step::Autofocus { range_um: 50.0 }],
        on_error: ErrorPolicy::default(),
    };
    let report = run(&devices, &target, &plan, &RunControl::new(), &|_| {})
        .await
        .unwrap();
    assert_eq!(report.outcome, Outcome::Failed);
    assert!(report.error.unwrap().contains("nothing to focus on"));

    // Plans that couldn't focus are refused before any dataset is made.
    let refused = [
        (Step::Autofocus { range_um: 0.0 }, "an autofocus over 0 µm"),
        (
            Step::FocusMap {
                places: vec![[0.0, 0.0], [100.0, 0.0]],
                surface: SurfaceKind::Plane,
                range_um: 50.0,
            },
            "a focus map of 2 places",
        ),
        (
            Step::FocusOnSurface { x: 0.0, y: 0.0 },
            "a focus on the surface before any focus map",
        ),
    ];
    for (step, why) in refused {
        let plan = Plan {
            name: "refused".to_owned(),
            steps: vec![step],
            on_error: ErrorPolicy::default(),
        };
        let refusal = run(&devices, &target, &plan, &RunControl::new(), &|_| {}).await;
        assert!(
            matches!(&refusal, Err(RunError::Invalid(error)) if error == why),
            "{refusal:?}"
        );
    }
}

#[test]
fn focus_surfaces_fit_their_places() {
    let lattice: Vec<[f64; 2]> = (0..3)
        .flat_map(|j| (0..3).map(move |i| [f64::from(i) * 200.0, f64::from(j) * 150.0]))
        .collect();
    // A plane, exactly; and with a place 20 µm off it, which is set aside.
    let tilted = |[x, y]: [f64; 2]| 1.0 + 0.01 * x - 0.02 * y;
    let mut places: Vec<[f64; 3]> = lattice.iter().map(|&p| [p[0], p[1], tilted(p)]).collect();
    let plane = fit_surface(&places, SurfaceKind::Plane).unwrap();
    assert!(plane.residuals.iter().all(|r| r.abs() < 1e-9));
    places[4][2] += 20.0;
    let plane = fit_surface(&places, SurfaceKind::Plane).unwrap();
    assert_eq!(plane.outliers, [4]);
    assert!((plane.surface.z(100.0, 75.0) - tilted([100.0, 75.0])).abs() < 1e-6);

    // A tilted and curved surface: the spline goes through its places and stays near it between
    // them; a plane can't follow the curve.
    let curved =
        |[x, y]: [f64; 2]| 0.02 * x - 0.01 * y + 1e-4 * ((x - 200.0).powi(2) + (y - 150.0).powi(2));
    let places: Vec<[f64; 3]> = lattice.iter().map(|&p| [p[0], p[1], curved(p)]).collect();
    let spline = fit_surface(&places, SurfaceKind::ThinPlate).unwrap();
    for p in &places {
        assert!((spline.surface.z(p[0], p[1]) - p[2]).abs() < 1e-6);
    }
    for at in [[100.0, 75.0], [300.0, 225.0], [50.0, 280.0]] {
        let off = spline.surface.z(at[0], at[1]) - curved(at);
        assert!(off.abs() < 1.0, "{off} µm off at {at:?}");
    }
    assert!(
        spline.residuals.iter().any(|r| r.abs() > 1e-6),
        "left-one-out residuals"
    );
    let flat = fit_surface(&places, SurfaceKind::Plane).unwrap();
    assert!(flat.residuals.iter().map(|r| r.abs()).fold(0.0, f64::max) > 1.0);
    assert!(fit_surface(&places[..2], SurfaceKind::ThinPlate).is_err());
}

#[tokio::test]
async fn a_5_by_5_scan_with_a_9_point_focus_map_is_in_focus_at_every_tile() {
    let specimen = Arc::new(Specimen::new());
    let world: Arc<dyn World> = specimen.clone();
    let common::Rig {
        devices,
        state,
        target,
        ..
    } = common::instrument((256, 192), Some(world)).await;
    // 5 × 5 tiles at 10 % overlap: 4.6 steps of 0.9 of a frame each way.
    let c = specimen.centre;
    let step = [0.9 * 256.0 * 0.376 / 1.002, 0.9 * 192.0 * 0.376 / 1.002];
    let half = [2.3 * step[0], 2.3 * step[1]];
    let region = Region::Rect {
        x0: c[0] - half[0],
        y0: c[1] - half[1],
        x1: c[0] + half[0],
        y1: c[1] + half[1],
    };
    // Across the scan the specimen's focus moves by more than a quarter of the depth of field.
    let corner = specimen.in_focus([c[0] + half[0], c[1] - half[1]], 0.0);
    assert!(
        (corner - specimen.in_focus(c, 0.0)).abs() > dof(),
        "a specimen worth mapping"
    );
    // The map's places: the middle, the edges' middles and the corners, at the tiles' centres.
    let places: Vec<[f64; 2]> = (-1..=1)
        .flat_map(|j| {
            (-1..=1).map(move |i| {
                [
                    c[0] + f64::from(i) * 2.0 * step[0],
                    c[1] + f64::from(j) * 2.0 * step[1],
                ]
            })
        })
        .collect();
    let plan = Plan {
        name: "mapped-scan".to_owned(),
        steps: vec![
            Step::FocusMap {
                places,
                surface: SurfaceKind::ThinPlate,
                range_um: 50.0,
            },
            Step::TileScan {
                region,
                overlap: 0.1,
                order: Order::Snake,
                channels: Vec::new(),
                settle_ms: 0.0,
                focus: TileFocus::Surface,
                z_stack: None,
            },
        ],
        on_error: ErrorPolicy::default(),
    };
    // At each frame, the focus, and where the specimen was in focus by the simulator's state.
    let taken = Mutex::new(Vec::new());
    let observe = |event: RunEvent| {
        if let RunEvent::Frame { p, .. } = event {
            let focus = devices.focus.status().borrow().position.0;
            taken
                .lock()
                .unwrap()
                .push((p, focus, specimen.in_focus_now(&state)));
        }
    };
    let report = run(&devices, &target, &plan, &RunControl::new(), &observe)
        .await
        .unwrap();
    assert_eq!(
        (report.outcome, report.frames),
        (Outcome::Finished, 25),
        "{report:?}"
    );
    let taken = taken.into_inner().unwrap();
    let worst = taken
        .iter()
        .map(|(_, focus, truth)| (focus - truth).abs())
        .fold(0.0, f64::max);
    assert!(
        worst < dof() / 4.0,
        "a tile {worst:.2} µm out, against {:.2}",
        dof() / 4.0
    );
    let layout = std::fs::read_to_string(report.path.join(LAYOUT)).unwrap();
    let layout: serde_json::Value = serde_json::from_str(&layout).unwrap();
    assert_eq!(
        layout["focus_maps"][0]["found"].as_array().unwrap().len(),
        9
    );
    assert_eq!(layout["scans"][0]["tiles"].as_array().unwrap().len(), 25);
}
