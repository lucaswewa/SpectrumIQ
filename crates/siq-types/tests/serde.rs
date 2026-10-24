//! The wire and file formats of the shared types: serde round trips, and snapshots of the frame
//! metadata's JSON and of the JSON Schemas, so a format change is always deliberate.

use proptest::prelude::*;
use siq_types::*;
use uuid::Uuid;

fn sample_meta() -> FrameMeta {
    FrameMeta {
        seq: FrameSeq(41),
        device_ts: Some(DeviceMicros(1_234_567)),
        host_ts_mid_exposure: Timestamp(987_654_321),
        exposure: Millis(12.5),
        gain: 1.0,
        roi: Roi::full(11_648, 8_742),
        binning: Binning::NONE,
        pixel_format: PixelFormat::Mono14,
        pose: Some(StagePose {
            x: Micrometers(1500.0),
            y: Micrometers(-250.25),
            z_coarse: Micrometers(0.0),
            focus: Micrometers(37.5),
        }),
        pose_quality: PoseQuality::Interpolated {
            uncertainty: Micrometers(0.8),
        },
        channel: Some(ChannelId(2)),
        objective: Some(ObjectiveId(1)),
        calibration: Some(CalibrationId::from_uuid(Uuid::from_u128(
            0x0199_0000_0000_7000_8000_0000_0000_0001,
        ))),
        flags: FrameFlags {
            save: true,
            moving: false,
        },
    }
}

// REQ-ACQ-004, REQ-CAL-003
#[test]
fn frame_metadata_json_is_stable() {
    insta::assert_json_snapshot!("frame_meta", sample_meta());
    assert_eq!(sample_meta().image_size(), (11_648, 8_742));
}

// REQ-ACQ-004
#[test]
fn json_schemas_are_stable() {
    insta::assert_json_snapshot!("schema_frame_meta", schemars::schema_for!(FrameMeta));
    insta::assert_json_snapshot!("schema_affine2", schemars::schema_for!(Affine2));
    insta::assert_json_snapshot!("schema_affine_params", schemars::schema_for!(AffineParams));
    insta::assert_json_snapshot!("schema_rect_um", schemars::schema_for!(RectUm));
    insta::assert_json_snapshot!("schema_dataset_id", schemars::schema_for!(DatasetId));
}

#[test]
fn units_describe_themselves_in_the_schema() {
    let schema = serde_json::to_value(schemars::schema_for!(Micrometers)).unwrap();
    assert_eq!(schema["type"], "number");
    let description = schema["description"].as_str().expect("a description");
    assert!(description.contains("µm"), "{description}");
}

#[test]
fn ids_print_parse_and_shorten() {
    let id = DatasetId::generate();
    assert_eq!(id.to_string().parse::<DatasetId>().unwrap(), id);
    assert_eq!(id.as_uuid().get_version_num(), 7);
    assert_eq!(id.short().len(), 8);
    assert!(id.as_uuid().simple().to_string().ends_with(&id.short()));
    assert!(DatasetId::generate() > id);
    assert!("not a uuid".parse::<CalibrationId>().is_err());
    assert_eq!(serde_json::to_string(&ChannelId(3)).unwrap(), "3");
    assert_eq!(ObjectiveId(4).to_string(), "4");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, format!("\"{id}\""));
}

fn micrometers() -> impl Strategy<Value = Micrometers> {
    (-1e6f64..1e6).prop_map(Micrometers)
}

fn pose() -> impl Strategy<Value = StagePose> {
    (micrometers(), micrometers(), micrometers(), micrometers()).prop_map(
        |(x, y, z_coarse, focus)| StagePose {
            x,
            y,
            z_coarse,
            focus,
        },
    )
}

fn pose_quality() -> impl Strategy<Value = PoseQuality> {
    prop_oneof![
        Just(PoseQuality::Settled),
        Just(PoseQuality::Unknown),
        micrometers().prop_map(|uncertainty| PoseQuality::Interpolated { uncertainty }),
    ]
}

fn pixel_format() -> impl Strategy<Value = PixelFormat> {
    prop_oneof![
        Just(PixelFormat::Mono8),
        Just(PixelFormat::Mono10),
        Just(PixelFormat::Mono12),
        Just(PixelFormat::Mono14),
        Just(PixelFormat::Mono16),
    ]
}

prop_compose! {
    fn frame_meta()(
        seq in any::<u64>(),
        device_ts in proptest::option::of(any::<u64>()),
        host_ts in any::<u64>(),
        (exposure, gain) in (0.0f64..1e5, 0.0f64..100.0),
        roi in any::<[u32; 4]>(),
        binning in (1u8..=255, 1u8..=255),
        pixel_format in pixel_format(),
        pose in proptest::option::of(pose()),
        pose_quality in pose_quality(),
        (channel, objective) in (proptest::option::of(any::<u16>()), proptest::option::of(any::<u16>())),
        calibration in proptest::option::of(any::<u128>()),
        (save, moving) in (any::<bool>(), any::<bool>()),
    ) -> FrameMeta {
        FrameMeta {
            seq: FrameSeq(seq),
            device_ts: device_ts.map(DeviceMicros),
            host_ts_mid_exposure: Timestamp(host_ts),
            exposure: Millis(exposure),
            gain,
            roi: Roi { offset_x: roi[0], offset_y: roi[1], width: roi[2], height: roi[3] },
            binning: Binning::new(binning.0, binning.1).expect("non-zero"),
            pixel_format,
            pose,
            pose_quality,
            channel: channel.map(ChannelId),
            objective: objective.map(ObjectiveId),
            calibration: calibration.map(|bits| CalibrationId::from_uuid(Uuid::from_u128(bits))),
            flags: FrameFlags { save, moving },
        }
    }
}

proptest! {
    // REQ-ACQ-004
    #[test]
    fn frame_metadata_round_trips_through_json(meta in frame_meta()) {
        let json = serde_json::to_string(&meta).unwrap();
        prop_assert_eq!(serde_json::from_str::<FrameMeta>(&json).unwrap(), meta);
    }

    #[test]
    fn transforms_round_trip_through_json(rows in any::<[[i32; 3]; 2]>()) {
        let affine = Affine2::new(rows.map(|row| row.map(|v| f64::from(v) / 7.0)));
        let json = serde_json::to_string(&affine).unwrap();
        prop_assert_eq!(serde_json::from_str::<Affine2>(&json).unwrap(), affine);
    }
}
