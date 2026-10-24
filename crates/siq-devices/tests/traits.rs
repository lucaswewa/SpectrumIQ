//! The traits are dyn-compatible with `Send` futures, and the descriptors check what they
//! describe.

use std::sync::Arc;

use siq_devices::mock::{
    MockCamera, MockChanger, MockFocusDrive, MockIllumination, MockModalityUnit, MockStage,
    small_sensor,
};
use siq_devices::*;
use siq_types::{Binning, Hertz, ManualClock, Micrometers, Millis, ModalityMode, PixelFormat, Roi};

/// The rig an engine holds: every device behind its trait.
struct Rig {
    camera: Arc<dyn Camera>,
    stage: Arc<dyn Stage>,
    focus: Arc<dyn FocusDrive>,
    illumination: Arc<dyn Illumination>,
    filters: Arc<dyn FilterWheel>,
    turret: Arc<dyn ObjectiveTurret>,
    modality: Arc<dyn ModalityUnit>,
}

fn rig() -> Rig {
    let clock = Arc::new(ManualClock::new());
    let names = |names: &[&str]| names.iter().map(|&name| name.to_owned()).collect();
    let travel = Travel {
        min: Xyz::from_um(0.0, 0.0, -5_000.0),
        max: Xyz::from_um(100_000.0, 75_000.0, 0.0),
    };
    let range = Travel {
        min: Micrometers(-500.0),
        max: Micrometers(500.0),
    };
    Rig {
        camera: Arc::new(MockCamera::new(small_sensor(), clock.clone())),
        stage: Arc::new(MockStage::new(travel, clock.clone())),
        focus: Arc::new(MockFocusDrive::new(range, clock)),
        illumination: Arc::new(MockIllumination::new(names(&["white", "470nm"]))),
        filters: Arc::new(MockChanger::new(names(&["empty", "GFP", "DAPI"]))),
        turret: Arc::new(MockChanger::new(names(&["4x", "10x", "40x"]))),
        modality: Arc::new(MockModalityUnit::new(vec![ModalityMode::Widefield], None)),
    }
}

// REQ-DEV-001, REQ-STG-003
#[tokio::test]
async fn every_trait_is_dyn_compatible_and_its_futures_are_send() {
    let rig = rig();
    // `spawn` needs `Send + 'static` futures: each one is a device operation behind its trait.
    let (stage, focus) = (rig.stage.clone(), rig.focus.clone());
    let moved = tokio::spawn(async move {
        let options = MoveOpts::default();
        let xy = stage
            .move_to(Xyz::from_um(10.0, 20.0, -1.0), options)
            .await?;
        let z = focus.move_by(Micrometers(2.5), options).await?;
        Ok::<_, DeviceError>((xy.position, z.position))
    });
    let (xy, z) = moved.await.unwrap().unwrap();
    assert_eq!((xy, z), (Xyz::from_um(10.0, 20.0, -1.0), Micrometers(2.5)));

    let camera = rig.camera.clone();
    tokio::spawn(async move { camera.stop().await })
        .await
        .unwrap()
        .unwrap();
    rig.illumination.set_level(1, 0.5).await.unwrap();
    rig.filters.select(2).await.unwrap();
    rig.turret.select(1).await.unwrap();
    rig.modality
        .set_modality(Modality::WIDEFIELD)
        .await
        .unwrap();

    // A filter wheel and a turret are both changers.
    let changers: [Arc<dyn Changer>; 2] = [rig.filters, rig.turret];
    let selected: Vec<_> = changers
        .iter()
        .map(|c| c.status().borrow().position)
        .collect();
    assert_eq!(selected, [Some(2), Some(1)]);
    assert_eq!(rig.camera.info().vendor, "SpectrumIQ");
}

fn full_frame() -> CameraSettings {
    CameraSettings::full_frame(&small_sensor(), Millis(1.0))
}

// REQ-ACQ-003
#[test]
fn camera_settings_are_checked_against_the_capabilities() {
    let capabilities = small_sensor();
    let base = full_frame();
    assert_eq!(capabilities.check(&base), Ok(()));
    let roi = |offset_x, offset_y, width, height| Roi {
        offset_x,
        offset_y,
        width,
        height,
    };
    let binned = CameraSettings {
        binning: Binning::new(2, 2).unwrap(),
        roi: roi(8, 4, 32, 16),
        ..base
    };
    assert_eq!(capabilities.check(&binned), Ok(()));
    assert_eq!(binned.image_size(), (16, 8));
    assert_eq!(binned.frame_bytes(), 16 * 8 * 2);

    let refused = [
        (
            "format",
            CameraSettings {
                pixel_format: PixelFormat::Mono12,
                ..base
            },
        ),
        (
            "binning",
            CameraSettings {
                binning: Binning::new(4, 4).unwrap(),
                ..base
            },
        ),
        (
            "outside",
            CameraSettings {
                roi: roi(32, 0, 48, 48),
                ..base
            },
        ),
        (
            "unaligned",
            CameraSettings {
                roi: roi(2, 0, 32, 32),
                ..base
            },
        ),
        (
            "too small",
            CameraSettings {
                roi: roi(0, 0, 4, 4),
                ..base
            },
        ),
        (
            "exposure",
            CameraSettings {
                exposure: Millis(20_000.0),
                ..base
            },
        ),
        (
            "gain",
            CameraSettings {
                gain: f64::NAN,
                ..base
            },
        ),
    ];
    for (case, settings) in refused {
        let result = capabilities.check(&settings);
        assert!(
            matches!(result, Err(DeviceError::InvalidArgument(_))),
            "{case}: {result:?}"
        );
    }
    // With single-pixel steps, an ROI can be a fraction of a binned pixel.
    let loose = CameraCapabilities {
        roi: RoiRules {
            step_x: 1,
            step_y: 1,
            min_width: 1,
            min_height: 1,
        },
        ..small_sensor()
    };
    let odd = CameraSettings {
        roi: roi(0, 0, 13, 12),
        ..binned
    };
    assert!(matches!(
        loose.check(&odd),
        Err(DeviceError::InvalidArgument(_))
    ));
}

// REQ-ACQ-003
#[test]
fn the_frame_rate_model_rises_for_an_roi_and_binning_and_falls_for_long_exposures() {
    let model = small_sensor().frame_rate;
    let rate = |settings: &CameraSettings| model.max_rate(settings).get();
    let full = full_frame();
    assert!((rate(&full) - 1000.0 / (0.4 + 0.2 * 48.0)).abs() < 1e-9);
    let roi = CameraSettings {
        roi: Roi {
            offset_x: 0,
            offset_y: 0,
            width: 64,
            height: 16,
        },
        ..full
    };
    let binned = CameraSettings {
        binning: Binning::new(2, 2).unwrap(),
        ..full
    };
    assert!(rate(&roi) > rate(&binned) && rate(&binned) > rate(&full));
    let long = CameraSettings {
        exposure: Millis(500.0),
        ..full
    };
    assert_eq!(model.max_rate(&long), Hertz(2.0));
}

#[test]
fn travel_and_modality_descriptors_check_what_they_describe() {
    let stage = Travel {
        min: Xyz::from_um(0.0, 0.0, -10.0),
        max: Xyz::from_um(100.0, 50.0, 0.0),
    };
    assert!(
        stage.contains(Xyz::from_um(100.0, 0.0, -10.0)),
        "both ends are in"
    );
    assert!(!stage.contains(Xyz::from_um(50.0, 50.1, -5.0)));
    let focus = Travel {
        min: Micrometers(-1.0),
        max: Micrometers(1.0),
    };
    assert!(focus.contains(Micrometers(0.0)) && !focus.contains(Micrometers(f64::NAN)));

    let modes = [
        ModalityMode::Widefield,
        ModalityMode::StructuredIllumination,
    ];
    let patterns = Some(PatternSet {
        orientations: 3,
        phases: 5,
    });
    let structured = |orientation, phase| Modality {
        mode: ModalityMode::StructuredIllumination,
        pattern: Some(Pattern { orientation, phase }),
    };
    assert_eq!(structured(2, 4).check(&modes, patterns), Ok(()));
    assert_eq!(Modality::WIDEFIELD.check(&modes, patterns), Ok(()));
    let refused = [
        (structured(3, 0), patterns),
        (structured(0, 0), None),
        (
            Modality {
                pattern: None,
                ..structured(0, 0)
            },
            patterns,
        ),
        (
            Modality {
                pattern: Some(Pattern {
                    orientation: 0,
                    phase: 0,
                }),
                ..Modality::WIDEFIELD
            },
            patterns,
        ),
        (
            Modality {
                mode: ModalityMode::Confocal,
                pattern: None,
            },
            patterns,
        ),
    ];
    for (modality, patterns) in refused {
        assert!(modality.check(&modes, patterns).is_err(), "{modality:?}");
    }
}
