//! The preview pipelines: geometry, display mapping, auto-contrast, the flat-field correction and
//! statistics on synthetic frames, mono and colour; then the running streams on the simulated
//! camera, for their rate caps and latency.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proptest::prelude::*;
use siq_acquire::Acquisition;
use siq_buffer::{Backend, PoolConfig};
use siq_calib::shading::FlatField;
use siq_devices::{CameraSettings, TriggerMode};
use siq_imgproc::{Colormap, DisplayMapping, Lut};
use siq_preview::{
    Contrast, DisplaySettings, FrameStats, Mapping, Measuring, PreviewConfig, PreviewSink,
    Previews, Renderer, RoiSettings, StatsSink, StreamKind, WhiteBalance, Zoom, fit, measure,
    roi_region,
};
use siq_sim::camera::{Lens, SimCamera, SimCameraSetup};
use siq_sim::optics::{OpticsModel, SensorModel};
use siq_sim::state::TrueState;
use siq_synth::{Cells, CellsParams};
use siq_telemetry::{MetricValue, Registry};
use siq_types::{
    Affine2, Binning, Cfa, FrameMeta, Hertz, ManualClock, Micrometers, Millis, OwnedImage,
    PixelFormat, RectPx, Roi, Timestamp,
};

fn meta(roi: Roi, binning: Binning) -> FrameMeta {
    let settings = CameraSettings {
        roi,
        binning,
        pixel_format: PixelFormat::Mono14,
        exposure: Millis(10.0),
        gain: 1.0,
        trigger: TriggerMode::FreeRun,
    };
    settings.frame_meta(0, Timestamp(0))
}

/// The width and height in a JPEG's frame header.
fn jpeg_size(jpeg: &[u8]) -> (u32, u32) {
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "a JPEG");
    let at = jpeg
        .windows(2)
        .position(|w| w[0] == 0xFF && w[1] == 0xC0)
        .expect("a frame header");
    let height = u16::from_be_bytes([jpeg[at + 5], jpeg[at + 6]]);
    let width = u16::from_be_bytes([jpeg[at + 7], jpeg[at + 8]]);
    (u32::from(width), u32::from(height))
}

#[test]
fn the_overview_fits_its_box_and_keeps_the_frame_s_aspect() {
    assert_eq!(fit((11_648, 8_742), (1920, 1080)), (1439, 1080));
    assert_eq!(fit((5824, 1000), (1920, 1080)), (1920, 330));
    assert_eq!(fit((640, 480), (1920, 1080)), (640, 480), "never enlarged");

    let frame = OwnedImage::<u16>::from_fn(1000, 700, |x, y| ((x * 7 + y * 13) % 16_384) as u16);
    let mut renderer = Renderer::default();
    let mapping = renderer.mapping(&frame.view(), 14, None, &DisplaySettings::default());
    let size = fit((1000, 700), (400, 400));
    renderer
        .show(
            &frame.view(),
            RectPx::new(0, 0, 1000, 700),
            size,
            mapping,
            None,
        )
        .unwrap();
    assert_eq!(size, (400, 280));
    assert_eq!(jpeg_size(&renderer.encode(85).unwrap()), size);
}

#[test]
fn a_shaded_frame_shows_flat_once_corrected_wherever_the_region_is() {
    // A frame of sensor columns 400.. and rows 200.., binned 2 × 2, lit `1 − 0.2 x² − 0.1 y²` over
    // a 2000 × 1500 sensor, over a dark level of 100.
    let flat = FlatField {
        sensor: [2000, 1500],
        coefficients: {
            let mut c = vec![0.0; 28];
            (c[0], c[2], c[13]) = (1.0, -0.2, -0.1);
            c
        },
    };
    let roi = Roi {
        offset_x: 400,
        offset_y: 200,
        width: 1200,
        height: 1000,
    };
    let frame_meta = meta(roi, Binning::new(2, 2).unwrap());
    let frame = OwnedImage::<u16>::from_fn(600, 500, |x, y| {
        let at = |i: usize, offset: u32| f64::from(offset) + (i as f64 + 0.5) * 2.0;
        let light = flat.at(at(x, roi.offset_x), at(y, roi.offset_y));
        (100.0 + 8000.0 * light).round() as u16
    });
    let mapping = DisplayMapping {
        min: 0,
        max: 16_383,
        gamma: 1.0,
        colormap: Colormap::Grey,
    };
    let mut renderer = Renderer::default();
    let shading = (100.0, flat);
    let regions = [
        (RectPx::new(0, 0, 600, 500), (150, 125)),
        (RectPx::new(450, 20, 150, 100), (150, 100)),
    ];
    for (region, size) in regions {
        renderer
            .show(
                &frame.view(),
                region,
                size,
                Mapping::Mono(mapping),
                Some((&shading, &frame_meta)),
            )
            .unwrap();
        let (_, levels) = renderer.shown();
        let (low, high) = (levels.iter().min().unwrap(), levels.iter().max().unwrap());
        assert!(high - low <= 1, "{region:?}: {low} to {high}");
    }
    renderer
        .show(
            &frame.view(),
            RectPx::new(0, 0, 600, 500),
            (150, 125),
            Mapping::Mono(mapping),
            None,
        )
        .unwrap();
    let (_, levels) = renderer.shown();
    assert!(
        levels.iter().max().unwrap() - levels.iter().min().unwrap() > 10,
        "uncorrected"
    );
}

#[test]
fn a_roi_maps_through_the_frame_s_region_and_binning_and_stays_inside_it() {
    // A frame of sensor columns 1000.. and rows 500.., binned 2 × 2: 2000 × 1000 frame pixels.
    let frame = meta(
        Roi {
            offset_x: 1000,
            offset_y: 500,
            width: 4000,
            height: 2000,
        },
        Binning::new(2, 2).unwrap(),
    );
    let at = |x, y, width, height, zoom| {
        let settings = RoiSettings {
            region: Some(RectPx::new(x, y, width, height)),
            zoom,
        };
        roi_region(&frame, &settings, (1920, 1080))
    };
    // Sensor (2000, 1000) is frame (500, 250).
    assert_eq!(
        at(2000, 1000, 400, 200, Zoom::Native),
        (RectPx::new(500, 250, 200, 100), (200, 100))
    );
    // Half the region off the frame's left edge: what is inside.
    assert_eq!(
        at(0, 1000, 1400, 200, Zoom::Native),
        (RectPx::new(0, 250, 200, 100), (200, 100))
    );
    // Too large: cut to the most shown, about its middle, and at 2:1 twice as much.
    assert_eq!(
        at(1000, 500, 4000, 2000, Zoom::Native),
        (RectPx::new(40, 0, 1920, 1000), (1920, 1000))
    );
    assert_eq!(
        at(1000, 500, 4000, 2000, Zoom::Half),
        (RectPx::new(0, 0, 2000, 1000), (1000, 500))
    );
    // Nothing of it in the frame, or nothing asked: the middle of the frame.
    let middle = (RectPx::new(40, 0, 1920, 1000), (1920, 1000));
    assert_eq!(at(0, 0, 500, 500, Zoom::Native), middle);
    assert_eq!(
        roi_region(&frame, &RoiSettings::default(), (1920, 1080)),
        middle
    );
}

proptest! {
    #[test]
    fn any_roi_is_inside_the_frame_aligned_to_its_zoom_and_within_the_largest_size(
        (offset_x, offset_y) in (0u32..2000, 0u32..2000),
        (width, height) in (64u32..6000, 64u32..4000),
        bin in 1u8..=2,
        region in (0u32..9000, 0u32..7000, 0u32..9000, 0u32..7000),
        half in any::<bool>(),
        max in (16u32..2000, 16u32..1200),
    ) {
        let frame = meta(Roi { offset_x, offset_y, width, height }, Binning::new(bin, bin).unwrap());
        let (frame_width, frame_height) = frame.image_size();
        let zoom = if half { Zoom::Half } else { Zoom::Native };
        let z = if half { 2 } else { 1 };
        let settings = RoiSettings { region: Some(RectPx::new(region.0, region.1, region.2, region.3)), zoom };
        let (rect, out) = roi_region(&frame, &settings, max);
        prop_assert!(rect.width >= 1 && rect.height >= 1);
        prop_assert!(rect.x + rect.width <= frame_width && rect.y + rect.height <= frame_height);
        prop_assert!(out.0 <= max.0 && out.1 <= max.1, "{out:?} in {max:?}");
        prop_assert_eq!(out, ((rect.width / z).max(1), (rect.height / z).max(1)));
        if rect.width + z <= frame_width {
            prop_assert_eq!((rect.x % z, rect.width % z), (0, 0), "{:?}", rect);
        }
    }
}

#[test]
fn a_lut_maps_the_window_through_its_gamma_and_colormap() {
    let frame = OwnedImage::<u16>::from_fn(256, 4, |x, _| (x * 64) as u16);
    let whole = RectPx::new(0, 0, 256, 4);
    let mut renderer = Renderer::default();
    for colormap in [
        Colormap::Grey,
        Colormap::Inverted,
        Colormap::Colour([0, 255, 128]),
    ] {
        let contrast = Contrast::Manual {
            min: 1000,
            max: 9000,
            gamma: 2.0,
        };
        let settings = DisplaySettings {
            contrast,
            colormap,
            white_balance: WhiteBalance::Auto,
        };
        let mapping = renderer.mapping(&frame.view(), 14, None, &settings);
        let expected = DisplayMapping {
            min: 1000,
            max: 9000,
            gamma: 2.0,
            colormap,
        };
        assert_eq!(mapping, Mapping::Mono(expected));
        renderer
            .show(&frame.view(), whole, (256, 4), mapping, None)
            .unwrap();
        let lut = Lut::new(&expected).unwrap();
        let ((width, height), bytes) = renderer.shown();
        assert_eq!((width, height), (256, 4));
        for x in [0, 15, 16, 100, 140, 141, 255] {
            let value = (x * 64) as u16;
            match colormap {
                Colormap::Colour(_) => assert_eq!(&bytes[x * 3..x * 3 + 3], &lut.colour(value)),
                _ => assert_eq!(bytes[x], lut.level(value)),
            }
        }
    }
}

#[test]
fn auto_contrast_takes_the_frame_s_percentiles() {
    // A frame of 0 to 9999, evenly: its 1st and 99th percentiles are about 100 and 9900.
    let frame = OwnedImage::<u16>::from_fn(100, 100, |x, y| (y * 100 + x) as u16);
    let mut renderer = Renderer::default();
    let contrast = Contrast::Auto {
        low_pct: 1.0,
        high_pct: 99.0,
    };
    let settings = DisplaySettings {
        contrast,
        ..DisplaySettings::default()
    };
    let Mapping::Mono(mapping) = renderer.mapping(&frame.view(), 14, None, &settings) else {
        panic!("a mono frame's mapping")
    };
    assert!((96..=104).contains(&mapping.min), "{mapping:?}");
    assert!((9896..=9904).contains(&mapping.max), "{mapping:?}");
    assert_eq!(mapping.gamma, 1.0);
    // A flat frame still has a window.
    let flat = OwnedImage::<u16>::from_fn(64, 64, |_, _| 500);
    let Mapping::Mono(mapping) = renderer.mapping(&flat.view(), 14, None, &settings) else {
        panic!("a mono frame's mapping")
    };
    assert!(mapping.min < mapping.max);
}

#[test]
fn statistics_count_the_region_and_measure_its_sharpness() {
    let textured =
        OwnedImage::<u16>::from_fn(
            200,
            100,
            |x, y| if (x / 4 + y / 4) % 2 == 0 { 1000 } else { 3000 },
        );
    let flat = OwnedImage::<u16>::from_fn(200, 100, |_, _| 2000);
    let frame = meta(Roi::full(200, 100), Binning::NONE);
    let region = RectPx::new(50, 25, 100, 50);
    let mut counts = Measuring::default();
    let sharp = measure(&textured.view(), &frame, 14, region, &mut counts).unwrap();
    assert_eq!(sharp.region, region);
    assert_eq!(
        sharp.histogram.iter().map(|&n| u64::from(n)).sum::<u64>(),
        5000
    );
    assert_eq!(
        sharp.histogram.iter().filter(|&&n| n > 0).count(),
        2,
        "two values"
    );
    assert!(
        sharp.window.0 <= 1000 && sharp.window.1 >= 3000,
        "{:?}",
        sharp.window
    );
    let blurred = measure(&flat.view(), &frame, 14, region, &mut counts).unwrap();
    assert!(sharp.focus > 0.0 && blurred.focus == 0.0);
    assert_eq!(sharp.colours, None, "a mono frame");
}

/// A colour frame of `width` × `height` whose filter array, `cfa`, passes red at 3000, green at
/// 4000 and blue at 2000.
fn colour_frame(cfa: Cfa, width: usize, height: usize) -> OwnedImage<u16> {
    OwnedImage::from_fn(width, height, |x, y| [3000, 4000, 2000][cfa.colour(x, y)])
}

#[test]
fn a_colour_frame_shows_in_its_colours_balanced_wherever_the_region_starts() {
    let frame = colour_frame(Cfa::Gbrg, 200, 100);
    let mut renderer = Renderer::default();
    let contrast = Contrast::Manual {
        min: 0,
        max: 4000,
        gamma: 1.0,
    };
    let shown = |renderer: &mut Renderer, white_balance, region, size| {
        let settings = DisplaySettings {
            contrast,
            colormap: Colormap::Grey,
            white_balance,
        };
        let mapping = renderer.mapping(&frame.view(), 14, Some(Cfa::Gbrg), &settings);
        renderer
            .show(&frame.view(), region, size, mapping, None)
            .unwrap();
        let ((width, height), bytes) = renderer.shown();
        assert_eq!(bytes.len(), (width * height * 3) as usize, "in colour");
        // Every pixel the same: the middle one's.
        let at = ((height / 2 * width + width / 2) * 3) as usize;
        let pixel = [bytes[at], bytes[at + 1], bytes[at + 2]];
        assert!(bytes.chunks(3).all(|p| p == pixel), "{region:?}");
        pixel
    };
    let raw = WhiteBalance::Fixed {
        red: 1.0,
        blue: 1.0,
    };
    // Shrunk from whole cells, and at 1:1 from an odd corner, whose cell starts with red.
    for (region, size) in [
        (RectPx::new(0, 0, 200, 100), (50, 25)),
        (RectPx::new(1, 1, 99, 49), (99, 49)),
    ] {
        assert_eq!(shown(&mut renderer, raw, region, size), [191, 255, 128]);
        let white = shown(&mut renderer, WhiteBalance::Auto, region, size);
        assert_eq!(white, [255; 3], "the brightest shows white");
    }
}

#[test]
fn a_colour_frame_s_statistics_are_per_colour_and_its_sharpness_on_luminance() {
    let frame = colour_frame(Cfa::Rggb, 200, 100);
    let mut meta = meta(Roi::full(200, 100), Binning::NONE);
    meta.cfa = Some(Cfa::Rggb);
    let region = RectPx::new(51, 25, 100, 50);
    let stats = measure(&frame.view(), &meta, 14, region, &mut Measuring::default()).unwrap();
    let colours = stats.colours.expect("per colour");
    let windows = colours.map(|(_, window)| window);
    for (window, value) in windows.into_iter().zip([3000, 4000, 2000]) {
        assert!(window.0 <= value && value <= window.1, "{windows:?}");
        assert!(window.1 - window.0 < 8, "{windows:?}");
    }
    assert_eq!(stats.window, windows[1], "green's");
    assert_eq!(stats.focus, 0.0, "one colour, flat");
}

/// Counts what each stream pushes, and keeps the last JPEG's size.
#[derive(Default)]
struct Counting {
    overview: AtomicUsize,
    roi: AtomicUsize,
    stats: AtomicUsize,
    last: Mutex<Option<(u32, u32)>>,
}

impl PreviewSink for Counting {
    fn push_jpeg(&self, stream: StreamKind, jpeg: Vec<u8>, meta: &FrameMeta) {
        assert!(meta.host_ts_mid_exposure > Timestamp(0));
        match stream {
            StreamKind::Overview => {
                self.overview.fetch_add(1, Ordering::Relaxed);
                *self.last.lock().unwrap() = Some(jpeg_size(&jpeg));
            }
            StreamKind::Roi => {
                self.roi.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl StatsSink for Counting {
    fn push_stats(&self, _: &FrameStats) {
        self.stats.fetch_add(1, Ordering::Relaxed);
    }
}

fn camera(clock: Arc<ManualClock>) -> SimCamera {
    let setup = SimCameraSetup {
        sensor: (512, 384),
        pixel_format: PixelFormat::Mono14,
        frame_rate: Hertz(9.0),
        pixel_size: Micrometers(3.76),
        lenses: vec![Lens {
            position: 0,
            magnification: 10.0,
            numerical_aperture: 0.30,
            immersion_index: 1.0,
        }],
        optics: OpticsModel::default(),
        sensor_model: SensorModel::default(),
        trigger_latency: Millis(0.1),
        mount: Affine2::IDENTITY,
    };
    SimCamera::new(
        setup,
        Arc::new(Cells::new(CellsParams::default())),
        Arc::new(TrueState::new(1)),
        clock,
    )
}

#[tokio::test]
async fn running_streams_keep_to_their_rate_caps_and_record_their_latency() {
    // The clock runs ten times faster than real time, so the camera offers 90 frames a second.
    let (clock, start) = (Arc::new(ManualClock::new()), Instant::now());
    let ticker = clock.clone();
    tokio::spawn(async move {
        loop {
            ticker.set(Timestamp(start.elapsed().as_nanos() as u64 * 10));
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots: 6,
        slot_bytes: 512 * 384 * 2,
        backend: Backend::Heap,
    };
    let acquisition =
        Acquisition::new(Arc::new(camera(clock.clone())), &pool, clock.clone()).unwrap();
    let config = PreviewConfig {
        overview_size: (256, 256),
        rates: [5.0, 20.0, 5.0],
        threads: 2,
        ..PreviewConfig::default()
    };
    let (sink, registry) = (Arc::new(Counting::default()), Registry::new());
    let previews = Previews::start(
        &acquisition,
        &config,
        sink.clone(),
        sink.clone(),
        clock,
        Some(&registry),
    )
    .unwrap();
    previews.set_roi(RoiSettings {
        region: Some(RectPx::new(100, 100, 200, 100)),
        zoom: Zoom::Native,
    });
    acquisition.start_live().await.unwrap();
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    acquisition.stop().await.unwrap();
    drop(previews);

    let overview = sink.overview.load(Ordering::Relaxed);
    let (roi, stats) = (
        sink.roi.load(Ordering::Relaxed),
        sink.stats.load(Ordering::Relaxed),
    );
    assert!(
        (3..=6).contains(&overview),
        "{overview} overviews at 5 a second"
    );
    assert!((3..=6).contains(&stats), "{stats} statistics at 5 a second");
    assert!(roi > overview && roi <= 21, "{roi} ROIs at 20 a second");
    assert_eq!(*sink.last.lock().unwrap(), Some((256, 192)));
    let snapshot = registry.snapshot();
    let Some(MetricValue::Histogram { count, sum, .. }) =
        snapshot.get("preview.overview.latency_ms")
    else {
        panic!("{snapshot:?}")
    };
    assert_eq!(*count as usize, overview);
    // Recorded, not asserted: the target, ≤ 60 ms at 100 MP, is for the release build on the
    // target PC. This clock runs ten times faster than real time.
    eprintln!(
        "overview latency: {:.1} ms a frame on the test clock",
        sum / *count as f64
    );
}
