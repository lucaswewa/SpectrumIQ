//! Sequences (ADR-0055) on the simulated instrument: a z-stack's planes in order, each approached
//! the same way, numbered from the bottom whichever way it goes; and a time-lapse of positions,
//! each scanned in tiles with z-stacks, its steps counted and every frame taken once.
//! REQ-WF-003, REQ-WF-004.

mod common;

use std::sync::{Arc, Mutex};

use siq_devices::{
    Approach, BoxFuture, Device, DeviceInfo, DeviceResult, FocusDrive, FocusStatus, MoveOpts,
    Travel, watch,
};
use siq_storage::{Dataset, DatasetKind, DatasetReader};
use siq_types::{Micrometers, MicrometersPerSecond};
use siq_workflow::DeviceSet;
use siq_workflow::engine::{
    ErrorPolicy, LAYOUT, Layout, Outcome, Overrun, Plan, Position, RunControl, RunEvent, RunReport,
    Step, TileFocus, ZDirection, ZPlanes, count, resolve, run,
};
use siq_workflow::scan::{Order, Region};

/// A focus drive that records each move it is asked for.
struct Recording {
    inner: Arc<dyn FocusDrive>,
    moves: Mutex<Vec<(f64, Approach)>>,
}

impl Device for Recording {
    fn info(&self) -> &DeviceInfo {
        self.inner.info()
    }
}

impl FocusDrive for Recording {
    fn move_to(
        &self,
        target: Micrometers,
        opts: MoveOpts,
    ) -> BoxFuture<'_, DeviceResult<FocusStatus>> {
        self.moves.lock().unwrap().push((target.0, opts.approach));
        self.inner.move_to(target, opts)
    }

    fn move_by(
        &self,
        delta: Micrometers,
        opts: MoveOpts,
    ) -> BoxFuture<'_, DeviceResult<FocusStatus>> {
        self.inner.move_by(delta, opts)
    }

    fn jog(&self, velocity: MicrometersPerSecond) -> BoxFuture<'_, DeviceResult<()>> {
        self.inner.jog(velocity)
    }

    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>> {
        self.inner.stop()
    }

    fn home(&self) -> BoxFuture<'_, DeviceResult<FocusStatus>> {
        self.inner.home()
    }

    fn status(&self) -> watch::Receiver<FocusStatus> {
        self.inner.status()
    }

    fn limits(&self) -> Travel {
        self.inner.limits()
    }
}

fn plan(steps: Vec<Step>) -> Plan {
    Plan {
        name: "sequence".to_owned(),
        steps,
        on_error: ErrorPolicy::default(),
    }
}

/// The raw frames' keys, `[t, z, p]`, in the order they were taken.
fn keys(report: &RunReport) -> Vec<[u32; 3]> {
    let dataset = Dataset::open(&report.path).unwrap();
    let reader = DatasetReader::open(&dataset).unwrap();
    let mut frames: Vec<_> = reader.frames().iter().map(|f| f.sidecar).collect();
    frames.sort_by_key(|frame| frame.meta.seq);
    frames.iter().map(|f| [f.key.t, f.key.z, f.key.p]).collect()
}

// REQ-WF-003
#[tokio::test]
async fn a_z_stack_takes_its_planes_in_order_each_approached_the_same_way() {
    let common::Rig {
        devices,
        state,
        target,
        ..
    } = common::instrument((256, 192), None).await;
    let recording = Arc::new(Recording {
        inner: devices.focus.clone(),
        moves: Mutex::new(Vec::new()),
    });
    let devices = DeviceSet {
        focus: recording.clone(),
        ..devices
    };
    for direction in [ZDirection::Up, ZDirection::Down] {
        // Five planes 2 µm apart about 10 µm.
        let planes = ZPlanes {
            range_um: 8.0,
            step_um: 2.0,
            direction,
        };
        let steps = vec![
            Step::Focus { z: 10.0 },
            Step::ZStack {
                planes,
                steps: Vec::new(),
            },
        ];
        assert_eq!(count(&steps), 1 + 5 * 2 + 1);
        recording.moves.lock().unwrap().clear();
        let taken = Mutex::new(Vec::new());
        let observe = |event| {
            if let RunEvent::Frame { z, .. } = event {
                taken.lock().unwrap().push((z, state.pose().focus.0));
            }
        };
        let report = run(
            &devices,
            &target,
            &plan(steps),
            &RunControl::new(),
            &observe,
        )
        .await
        .unwrap();
        assert_eq!((report.outcome, report.frames), (Outcome::Finished, 5));
        // Numbered from the bottom, taken the way asked.
        let mut planes = vec![(0, 6.0), (1, 8.0), (2, 10.0), (3, 12.0), (4, 14.0)];
        if direction == ZDirection::Down {
            planes.reverse();
        }
        assert_eq!(taken.into_inner().unwrap(), planes, "{direction:?}");
        // Every plane approached from the drive's side, then the focus back in the middle.
        let moves = recording.moves.lock().unwrap().clone();
        let mut expected = vec![(10.0, Approach::Unidirectional)];
        expected.extend(planes.iter().map(|&(_, z)| (z, Approach::Unidirectional)));
        expected.push((10.0, Approach::Unidirectional));
        assert_eq!(moves, expected, "{direction:?}");
        let kind = Dataset::open(&report.path).unwrap().manifest().kind;
        assert_eq!(kind, DatasetKind::ZStack);
    }
}

// REQ-WF-003, REQ-WF-004
#[tokio::test]
async fn a_time_lapse_of_positions_scanned_in_z_stacks_takes_every_frame_once() {
    let common::Rig {
        devices, target, ..
    } = common::instrument((256, 192), None).await;
    // 2 × 1 tiles about each position, at 10 % overlap.
    let step = [0.9 * 256.0 * 0.376 / 1.002, 0.9 * 192.0 * 0.376 / 1.002];
    let region = Region::Rect {
        x0: -0.8 * step[0],
        y0: -0.3 * step[1],
        x1: 0.8 * step[0],
        y1: 0.3 * step[1],
    };
    let at = |x: f64, y: f64| Position {
        name: String::new(),
        x,
        y,
        z: None,
        focus: None,
    };
    let scan = Step::TileScan {
        region,
        overlap: 0.1,
        order: Order::Snake,
        channels: Vec::new(),
        settle_ms: 0.0,
        focus: TileFocus::None,
        z_stack: Some(ZPlanes {
            range_um: 4.0,
            step_um: 2.0,
            direction: ZDirection::Up,
        }),
    };
    let positions = Step::MultiPosition {
        positions: vec![at(-20_000.0, -15_000.0), at(-19_500.0, -15_000.0)],
        autofocus: None,
        steps: vec![scan],
    };
    let steps = vec![Step::TimeLapse {
        interval_s: 1.0,
        count: Some(2),
        duration_s: None,
        overrun: Overrun::Shift,
        steps: vec![positions],
    }];
    // At each point, each position: a move, then at each tile a move and a stack of three
    // planes, each a focus and a frame, and the focus back.
    let resolved = resolve(&devices, &steps, &mut Layout::default()).unwrap();
    let total = 2 * 2 * (1 + 2 * (1 + 3 * 2 + 1));
    assert_eq!(count(&resolved), total);
    let report = run(&devices, &target, &plan(steps), &RunControl::new(), &|_| {})
        .await
        .unwrap();
    assert_eq!(
        (report.outcome, report.steps, report.frames),
        (Outcome::Finished, [total, total], 24),
        "{report:?}"
    );
    // Every frame once: two points, the four tiles of both positions, three planes each.
    let mut keys = keys(&report);
    keys.sort_unstable();
    let every: Vec<[u32; 3]> = (0..2)
        .flat_map(|t| (0..3).flat_map(move |z| (0..4).map(move |p| [t, z, p])))
        .collect();
    let mut every = every;
    every.sort_unstable();
    assert_eq!(keys, every);
    // The tiles about each position, numbered on from the first scan's.
    let text = std::fs::read_to_string(report.path.join(LAYOUT)).unwrap();
    let layout: Layout = serde_json::from_str(&text).unwrap();
    let firsts: Vec<u32> = layout.scans.iter().map(|scan| scan.first_p).collect();
    assert_eq!(firsts, [0, 2]);
    for (scan, x) in layout.scans.iter().zip([-20_000.0, -19_500.0]) {
        let mean = scan.grid.tiles.iter().map(|tile| tile.x).sum::<f64>() / 2.0;
        assert!((mean - x).abs() < 1.0, "tiles about {x}: {mean}");
    }
    assert!(layout.positions.iter().all(|placed| placed.p.is_none()));
    assert_eq!(layout.time_lapses[0].points.len(), 2);
    let kind = Dataset::open(&report.path).unwrap().manifest().kind;
    assert_eq!(kind, DatasetKind::TileScan);
}

proptest::proptest! {
    // REQ-WF-003
    #[test]
    fn a_z_stack_s_planes_are_a_step_apart_about_the_middle(
        range in 0.0..200.0f64,
        step in 0.5..20.0f64,
    ) {
        let planes = |direction| ZPlanes { range_um: range, step_um: step, direction }.planes();
        let (up, mut down) = (planes(ZDirection::Up), planes(ZDirection::Down));
        proptest::prop_assert_eq!(up.len(), (range / step + 1e-9).floor() as usize + 1);
        for (k, pair) in up.windows(2).enumerate() {
            proptest::prop_assert_eq!(pair[0].0 as usize, k);
            proptest::prop_assert!((pair[1].1 - pair[0].1 - step).abs() < 1e-9);
        }
        let (first, last) = (up[0].1, up[up.len() - 1].1);
        proptest::prop_assert!((first + last).abs() < 1e-9 && last - first <= range + 1e-9);
        down.reverse();
        proptest::prop_assert_eq!(down, up);
    }
}
