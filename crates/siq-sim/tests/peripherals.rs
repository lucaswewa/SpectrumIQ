//! The peripheral simulators, on a manual clock that the test moves: each change takes its time,
//! then sets the true state.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use siq_devices::{
    Changer, ChangerStatus, DeviceError, DeviceResult, Illumination, Modality, ModalityUnit,
    Pattern, PatternSet,
};
use siq_sim::peripherals::{SimChanger, SimIllumination, SimModalityUnit};
use siq_sim::state::TrueState;
use siq_types::{ManualClock, ModalityMode};

type Change<'a> = Pin<Box<dyn Future<Output = DeviceResult<()>> + Send + 'a>>;

/// Whether `change` is still under way after a little real time.
async fn pending(change: &mut Change<'_>) -> bool {
    tokio::time::timeout(Duration::from_millis(20), change)
        .await
        .is_err()
}

/// Checks that `change` takes `ms` on the clock: not done 1 ms before, done at it.
async fn takes(clock: &ManualClock, mut change: Change<'_>, ms: u64) {
    if ms > 0 {
        assert!(pending(&mut change).await, "done at once");
        clock.advance(Duration::from_millis(ms - 1));
        assert!(pending(&mut change).await, "done before {ms} ms");
        clock.advance(Duration::from_millis(1));
    }
    tokio::time::timeout(Duration::from_secs(1), change)
        .await
        .expect("done")
        .unwrap();
}

fn setup(sources: usize) -> (Arc<ManualClock>, Arc<TrueState>) {
    (
        Arc::new(ManualClock::new()),
        Arc::new(TrueState::new(sources)),
    )
}

#[tokio::test]
async fn a_light_settles_then_lights_the_true_state() {
    let (clock, state) = setup(2);
    let names = vec!["white".to_owned(), "470nm".to_owned()];
    let light = SimIllumination::new(names, state.clone(), clock.clone());
    assert_eq!(light.sources().len(), 2);
    takes(&clock, light.set_level(1, 0.5), 5).await;
    assert_eq!(
        (state.level(0), state.level(1)),
        (1.0, 0.5),
        "the others stay as they were"
    );
    assert_eq!(light.status().borrow().levels, [1.0, 0.5]);
    for refused in [light.set_level(2, 0.5).await, light.set_level(0, 1.5).await] {
        assert!(
            matches!(refused, Err(DeviceError::InvalidArgument(_))),
            "{refused:?}"
        );
    }
}

#[tokio::test]
async fn a_filter_wheel_turns_the_shorter_way_with_nothing_in_the_path_meanwhile() {
    let (clock, state) = setup(0);
    let filters = ["empty", "DAPI", "GFP", "TRITC"].map(String::from).to_vec();
    let wheel = SimChanger::filter_wheel(filters, state.clone(), clock.clone());
    assert_eq!(
        (state.filter(), wheel.status().borrow().position),
        (None, None)
    );

    // From an unknown position, half the way round: 2 positions of 60 ms.
    let mut change = wheel.select(2);
    assert!(pending(&mut change).await);
    let moving = ChangerStatus {
        position: None,
        moving: true,
    };
    assert_eq!((*wheel.status().borrow(), state.filter()), (moving, None));
    takes(&clock, change, 120).await;
    assert_eq!(
        (state.filter(), wheel.status().borrow().position),
        (Some(2), Some(2))
    );

    // 3 then 0 are one position each; 0 again is no move.
    for (position, ms) in [(3, 60), (0, 60), (0, 0), (2, 120)] {
        takes(&clock, wheel.select(position), ms).await;
        assert_eq!(state.filter(), Some(position));
    }
    let refused = wheel.select(4).await;
    assert!(
        matches!(refused, Err(DeviceError::InvalidArgument(_))),
        "{refused:?}"
    );
}

#[tokio::test]
async fn an_objective_turret_takes_400_ms_a_position() {
    let (clock, state) = setup(0);
    let turret = SimChanger::objective_turret(6, state.clone(), clock.clone());
    assert_eq!(turret.positions()[5], "position 5");
    assert_eq!(
        state.objective(),
        Some(0),
        "the true state starts at position 0"
    );
    takes(&clock, turret.select(1), 400).await;
    takes(&clock, turret.select(5), 2 * 400).await;
    assert_eq!(state.objective(), Some(5));
}

#[tokio::test]
async fn a_modality_unit_changes_mode_slowly_and_pattern_quickly() {
    let (clock, state) = setup(0);
    let modes = vec![
        ModalityMode::Widefield,
        ModalityMode::StructuredIllumination,
    ];
    let patterns = PatternSet {
        orientations: 3,
        phases: 3,
    };
    let unit = SimModalityUnit::new(modes, Some(patterns), state.clone(), clock.clone());
    assert_eq!(state.modality(), Modality::WIDEFIELD);
    let structured = |orientation, phase| Modality {
        mode: ModalityMode::StructuredIllumination,
        pattern: Some(Pattern { orientation, phase }),
    };
    takes(&clock, unit.set_modality(structured(0, 0)), 300).await;
    takes(&clock, unit.set_modality(structured(1, 2)), 20).await;
    takes(&clock, unit.set_modality(structured(1, 2)), 0).await;
    assert_eq!(state.modality(), structured(1, 2));
    assert!(!unit.status().borrow().moving);

    let confocal = Modality {
        mode: ModalityMode::Confocal,
        pattern: None,
    };
    for refused in [structured(3, 0), confocal] {
        let refused = unit.set_modality(refused).await;
        assert!(
            matches!(refused, Err(DeviceError::InvalidArgument(_))),
            "{refused:?}"
        );
    }
    assert_eq!(state.modality(), structured(1, 2));
}
