//! Every kernel against a naive reference: proptests on small images with padded rows, and one
//! large image each, which splits into several bands.

use proptest::prelude::*;
use siq_imgproc::*;
use siq_types::{Binning, ImageView, OwnedImage};

/// An image whose rows are `stride` apart.
#[derive(Debug, Clone)]
struct Padded<T> {
    width: usize,
    height: usize,
    stride: usize,
    data: Vec<T>,
}

impl<T: Copy> Padded<T> {
    fn from_fn(width: usize, height: usize, pixel: impl Fn(usize, usize) -> T, pad: T) -> Self {
        let stride = width + 3;
        let data = (0..stride * height)
            .map(|i| {
                if i % stride < width {
                    pixel(i % stride, i / stride)
                } else {
                    pad
                }
            })
            .collect();
        Self {
            width,
            height,
            stride,
            data,
        }
    }

    fn view(&self) -> ImageView<'_, T> {
        ImageView::new(&self.data, self.width, self.height, self.stride).unwrap()
    }

    fn at(&self, x: usize, y: usize) -> T {
        self.data[y * self.stride + x]
    }
}

fn padded<T: std::fmt::Debug + Clone>(
    values: impl Strategy<Value = T> + Clone,
    max_width: usize,
    max_height: usize,
) -> impl Strategy<Value = Padded<T>> {
    (1..=max_width, 1..=max_height, 0..4usize).prop_flat_map(move |(width, height, pad)| {
        let stride = width + pad;
        prop::collection::vec(values.clone(), stride * (height - 1) + width).prop_map(move |data| {
            Padded {
                width,
                height,
                stride,
                data,
            }
        })
    })
}

fn pixels<T: Copy>(image: &OwnedImage<T>) -> Vec<T> {
    image.view().rows().flatten().copied().collect()
}

fn close(found: &[f32], expected: &[f64], tolerance: f64) -> Result<(), TestCaseError> {
    prop_assert_eq!(found.len(), expected.len());
    for (i, (&f, &e)) in found.iter().zip(expected).enumerate() {
        prop_assert!(
            (f64::from(f) - e).abs() <= tolerance,
            "pixel {}: {} ≠ {}",
            i,
            f,
            e
        );
    }
    Ok(())
}

// --- References ---

fn correct_reference(
    raw: &Padded<u16>,
    dark: &Padded<u16>,
    gain: &Padded<f32>,
    max: u16,
) -> Vec<u16> {
    let pixel = |x, y| {
        let value = f32::from(raw.at(x, y).saturating_sub(dark.at(x, y))) * gain.at(x, y);
        (value.round().clamp(0.0, 65535.0) as u16).min(max)
    };
    (0..raw.height)
        .flat_map(|y| (0..raw.width).map(move |x| (x, y)))
        .map(|(x, y)| pixel(x, y))
        .collect()
}

fn overlap(i: usize, start: f64, end: f64) -> f64 {
    ((i as f64 + 1.0).min(end) - (i as f64).max(start)).max(0.0)
}

fn area_reference(input: &Padded<f32>, columns: usize, rows: usize) -> Vec<f64> {
    let (sx, sy) = (
        input.width as f64 / columns as f64,
        input.height as f64 / rows as f64,
    );
    let mut out = Vec::new();
    for oy in 0..rows {
        for ox in 0..columns {
            let mut sum = 0.0;
            // Every pixel the output pixel's area touches, and none it doesn't.
            let (x0, x1) = (ox as f64 * sx, (ox + 1) as f64 * sx);
            let (y0, y1) = (oy as f64 * sy, (oy + 1) as f64 * sy);
            for y in (y0.floor() as usize)..(y1.ceil() as usize).min(input.height) {
                for x in (x0.floor() as usize)..(x1.ceil() as usize).min(input.width) {
                    let weight = overlap(x, x0, x1) * overlap(y, y0, y1);
                    sum += weight * f64::from(input.at(x, y));
                }
            }
            out.push(sum / (sx * sy));
        }
    }
    out
}

fn bin_reference(input: &Padded<u16>, bx: usize, by: usize, mode: BinMode) -> Vec<u16> {
    let mut out = Vec::new();
    for oy in 0..input.height / by {
        for ox in 0..input.width / bx {
            let sum: u32 = (0..by)
                .flat_map(|dy| (0..bx).map(move |dx| (dx, dy)))
                .map(|(dx, dy)| u32::from(input.at(ox * bx + dx, oy * by + dy)))
                .sum();
            out.push(match mode {
                BinMode::Sum => sum.min(65535) as u16,
                BinMode::Mean => (f64::from(sum) / (bx * by) as f64).round() as u16,
            });
        }
    }
    out
}

fn blur_reference(input: &Padded<f32>, sigma: f64) -> Vec<f64> {
    let radius = (3.0 * sigma).ceil() as i64;
    let weight = |k: i64| {
        if sigma > 0.0 {
            (-((k * k) as f64) / (2.0 * sigma * sigma)).exp()
        } else {
            1.0
        }
    };
    let total: f64 = (-radius..=radius).map(weight).sum();
    let clamp = |v: i64, len: usize| v.clamp(0, len as i64 - 1) as usize;
    let mut out = Vec::new();
    for y in 0..input.height as i64 {
        for x in 0..input.width as i64 {
            let mut sum = 0.0;
            for j in -radius..=radius {
                for i in -radius..=radius {
                    let value =
                        f64::from(input.at(clamp(x + i, input.width), clamp(y + j, input.height)));
                    sum += weight(i) * weight(j) * value;
                }
            }
            out.push(sum / (total * total));
        }
    }
    out
}

fn bilinear_reference(input: &Padded<f32>, columns: usize, rows: usize) -> Vec<f64> {
    let source = |o: usize, out: usize, len: usize| {
        let at = ((o as f64 + 0.5) * len as f64 / out as f64 - 0.5).clamp(0.0, (len - 1) as f64);
        (
            at.floor() as usize,
            (at.floor() as usize + 1).min(len - 1),
            at - at.floor(),
        )
    };
    let mut out = Vec::new();
    for oy in 0..rows {
        let (y0, y1, fy) = source(oy, rows, input.height);
        for ox in 0..columns {
            let (x0, x1, fx) = source(ox, columns, input.width);
            let at = |x, y| f64::from(input.at(x, y));
            let (top, bottom) = (
                at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx,
                at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx,
            );
            out.push(top * (1.0 - fy) + bottom * fy);
        }
    }
    out
}

fn focus_reference(input: &Padded<u16>, measure: FocusMeasure) -> f64 {
    let at = |x: usize, y: usize| f64::from(input.at(x, y));
    let (w, h) = (input.width, input.height);
    let interior = || {
        (1..h.saturating_sub(1)).flat_map(move |y| (1..w.saturating_sub(1)).map(move |x| (x, y)))
    };
    let stats = |values: Vec<f64>| {
        let n = values.len() as f64;
        let mean = values.iter().sum::<f64>() / n;
        (
            mean,
            values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n,
        )
    };
    match measure {
        _ if (w < 3 || h < 3) && measure != FocusMeasure::NormalizedVariance => 0.0,
        FocusMeasure::LaplacianVariance => {
            stats(
                interior()
                    .map(|(x, y)| {
                        at(x - 1, y) + at(x + 1, y) + at(x, y - 1) + at(x, y + 1) - 4.0 * at(x, y)
                    })
                    .collect(),
            )
            .1
        }
        FocusMeasure::Tenengrad => {
            let values: Vec<f64> = interior()
                .map(|(x, y)| {
                    let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1))
                        - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
                    let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1))
                        - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
                    gx * gx + gy * gy
                })
                .collect();
            values.iter().sum::<f64>() / values.len() as f64
        }
        FocusMeasure::NormalizedVariance => {
            let (mean, variance) = stats(
                (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .map(|(x, y)| at(x, y))
                    .collect(),
            );
            if mean > 0.0 { variance / mean } else { 0.0 }
        }
    }
}

// --- Proptests ---

proptest! {
    // REQ-CAL-002
    #[test]
    fn correction_matches_the_reference(
        raw in padded(0..=16383u16, 40, 30),
        offsets in prop::collection::vec(0..=400u16, 1200),
        gains in prop::collection::vec(0.5f32..2.5, 1200),
        max in 1000..=16383u16,
    ) {
        let dark = Padded::from_fn(raw.width, raw.height, |x, y| offsets[y * 40 + x], 0);
        let gain = Padded::from_fn(raw.width, raw.height, |x, y| gains[y * 40 + x], 0.0);
        let calibration = Calibration { dark: Some(dark.view()), gain: Some(gain.view()) };
        let mut out = OwnedImage::new(raw.width, raw.height);
        correct(&raw.view(), &calibration, max, &mut out.view_mut()).unwrap();
        let expected = correct_reference(&raw, &dark, &gain, max);
        prop_assert_eq!(pixels(&out), expected.clone());

        let mut image = OwnedImage::from_fn(raw.width, raw.height, |x, y| raw.at(x, y));
        correct_in_place(&mut image.view_mut(), &calibration, max).unwrap();
        prop_assert_eq!(pixels(&image), expected);
    }

    #[test]
    fn an_area_average_matches_the_reference(
        (input, columns, rows) in padded(0.0f32..1.0, 40, 30)
            .prop_flat_map(|input| { let (w, h) = (input.width, input.height); (Just(input), 1..=w, 1..=h) }),
    ) {
        let mut out = OwnedImage::new(columns, rows);
        downsample_area(&input.view(), &mut out.view_mut()).unwrap();
        close(&pixels(&out), &area_reference(&input, columns, rows), 1e-5)?;
    }

    #[test]
    fn binning_matches_the_reference(
        input in padded(0..=u16::MAX, 40, 30),
        (bx, by) in (1..=4u8, 1..=4u8),
        sum in any::<bool>(),
    ) {
        let mode = if sum { BinMode::Sum } else { BinMode::Mean };
        let binning = Binning::new(bx, by).unwrap();
        let (w, h) = (input.width / usize::from(bx), input.height / usize::from(by));
        let mut out = OwnedImage::new(w, h);
        bin(&input.view(), binning, mode, &mut out.view_mut()).unwrap();
        prop_assert_eq!(pixels(&out), bin_reference(&input, usize::from(bx), usize::from(by), mode));
    }

    #[test]
    fn a_copy_and_a_crop_keep_every_pixel(input in padded(any::<u16>(), 40, 30)) {
        let mut out = OwnedImage::new(input.width, input.height);
        copy(&input.view(), &mut out.view_mut()).unwrap();
        let expected: Vec<u16> = (0..input.height).flat_map(|y| (0..input.width).map(move |x| (x, y))).map(|(x, y)| input.at(x, y)).collect();
        prop_assert_eq!(pixels(&out), expected);
    }

    #[test]
    fn a_blur_matches_the_reference(input in padded(0.0f32..1.0, 30, 20), sigma in 0.0f32..4.0) {
        let (mut scratch, mut out) = (OwnedImage::new(input.width, input.height), OwnedImage::new(input.width, input.height));
        gaussian_blur(&input.view(), sigma, &mut scratch.view_mut(), &mut out.view_mut()).unwrap();
        close(&pixels(&out), &blur_reference(&input, f64::from(sigma)), 1e-5)?;
    }

    #[test]
    fn bilinear_enlarging_matches_the_reference(
        (input, columns, rows) in padded(0.0f32..1.0, 20, 15)
            .prop_flat_map(|input| { let (w, h) = (input.width, input.height); (Just(input), w..=3 * w, h..=3 * h) }),
    ) {
        let mut out = OwnedImage::new(columns, rows);
        upsample_bilinear(&input.view(), &mut out.view_mut()).unwrap();
        close(&pixels(&out), &bilinear_reference(&input, columns, rows), 1e-5)?;
    }

    // REQ-PRV-003
    #[test]
    fn the_histogram_counts_the_subsample(input in padded(0..=16383u16, 40, 30), step in 1..=5usize) {
        let mut histogram = Histogram::new(14);
        siq_imgproc::histogram(&input.view(), step, &mut histogram).unwrap();
        let mut expected = [0u32; BINS];
        let mut sampled = Vec::new();
        for y in (0..input.height).step_by(step) {
            for x in (0..input.width).step_by(step) {
                expected[usize::from(input.at(x, y) >> 2)] += 1;
                sampled.push(input.at(x, y));
            }
        }
        prop_assert_eq!(histogram.bins(), &expected);
        prop_assert_eq!(histogram.count(), sampled.len() as u64);
        sampled.sort_unstable();
        for fraction in [0.0, 0.01, 0.5, 0.99, 1.0] {
            let rank = ((fraction * sampled.len() as f64).ceil() as usize).max(1) - 1;
            prop_assert_eq!(histogram.percentile(fraction), sampled[rank] & !3, "at {}", fraction);
        }
        let (low, high) = histogram.auto_window(0.01, 0.99);
        prop_assert!(low < high && high >= histogram.percentile(0.99));
    }

    // REQ-PRV-003
    #[test]
    fn the_lut_maps_every_pixel_as_its_formula(
        input in padded(any::<u16>(), 30, 20),
        (min, span) in (0..60_000u16, 1..5000u16),
        gamma in 0.3f32..3.0,
        colour in any::<[u8; 3]>(),
    ) {
        let mapping = DisplayMapping { min, max: min + span, gamma, colormap: Colormap::Colour(colour) };
        let lut = Lut::new(&mapping).unwrap();
        let level = |v: u16| ((((f32::from(v) - f32::from(min)) / f32::from(span)).clamp(0.0, 1.0)).powf(1.0 / gamma) * 255.0).round() as u8;
        let (mut grey, mut rgb) = (OwnedImage::new(input.width, input.height), OwnedImage::new(input.width, input.height));
        lut.apply(&input.view(), &mut grey.view_mut()).unwrap();
        lut.apply_rgb(&input.view(), &mut rgb.view_mut()).unwrap();
        let values: Vec<u16> = (0..input.height).flat_map(|y| (0..input.width).map(move |x| (x, y))).map(|(x, y)| input.at(x, y)).collect();
        prop_assert_eq!(pixels(&grey), values.iter().map(|&v| level(v)).collect::<Vec<_>>());
        let coloured = |v: u16| colour.map(|c| ((u32::from(c) * u32::from(level(v)) + 127) / 255) as u8);
        prop_assert_eq!(pixels(&rgb), values.iter().map(|&v| coloured(v)).collect::<Vec<_>>());
    }

    #[test]
    fn focus_measures_match_the_reference(input in padded(0..=16383u16, 30, 20)) {
        for measure in [FocusMeasure::LaplacianVariance, FocusMeasure::Tenengrad, FocusMeasure::NormalizedVariance] {
            let (found, expected) = (focus(&input.view(), measure), focus_reference(&input, measure));
            prop_assert!((found - expected).abs() <= 1e-6 * expected.abs().max(1.0), "{:?}: {} ≠ {}", measure, found, expected);
        }
    }
}

// --- Large images, in several bands ---

fn ramp(width: usize, height: usize) -> Padded<u16> {
    Padded::from_fn(
        width,
        height,
        |x, y| ((x * 7 + y * 13 + (x * y) % 97) % 16384) as u16,
        0,
    )
}

#[test]
fn large_images_split_into_bands_give_the_same_results() {
    let raw = ramp(2000, 400);
    let gain = Padded::from_fn(2000, 400, |x, _| 0.8 + (x % 5) as f32 * 0.1, 0.0);
    let dark = Padded::from_fn(2000, 400, |x, y| ((x + y) % 50) as u16, 0);
    let calibration = Calibration {
        dark: Some(dark.view()),
        gain: Some(gain.view()),
    };
    let mut out = OwnedImage::new(2000, 400);
    correct(&raw.view(), &calibration, 16383, &mut out.view_mut()).unwrap();
    assert_eq!(pixels(&out), correct_reference(&raw, &dark, &gain, 16383));

    let mut binned = OwnedImage::new(1000, 200);
    bin(
        &raw.view(),
        Binning::new(2, 2).unwrap(),
        BinMode::Sum,
        &mut binned.view_mut(),
    )
    .unwrap();
    assert_eq!(pixels(&binned), bin_reference(&raw, 2, 2, BinMode::Sum));

    let float = Padded::from_fn(2000, 400, |x, y| f32::from(raw.at(x, y)) / 16383.0, 0.0);
    let mut small = OwnedImage::new(333, 77);
    downsample_area(&float.view(), &mut small.view_mut()).unwrap();
    close(&pixels(&small), &area_reference(&float, 333, 77), 1e-5).unwrap();

    let (mut scratch, mut blurred) = (OwnedImage::new(2000, 400), OwnedImage::new(2000, 400));
    gaussian_blur(
        &float.view(),
        1.5,
        &mut scratch.view_mut(),
        &mut blurred.view_mut(),
    )
    .unwrap();
    let crop = |image: &OwnedImage<f32>| -> Vec<f32> {
        image
            .view()
            .rows()
            .skip(150)
            .take(100)
            .flat_map(|row| row[900..1000].to_vec())
            .collect()
    };
    let reference = blur_reference(&float, 1.5);
    let expected: Vec<f64> = (150..250)
        .flat_map(|y| (900..1000).map(move |x| (x, y)))
        .map(|(x, y)| reference[y * 2000 + x])
        .collect();
    close(&crop(&blurred), &expected, 1e-5).unwrap();

    let mut histogram = Histogram::new(14);
    siq_imgproc::histogram(&raw.view(), 3, &mut histogram).unwrap();
    assert_eq!(histogram.count(), 667 * 134);
    let reference = focus_reference(&raw, FocusMeasure::Tenengrad);
    let found = focus(&raw.view(), FocusMeasure::Tenengrad);
    assert!(
        (found - reference).abs() <= 1e-6 * reference,
        "{found} ≠ {reference}"
    );
}

// --- Edges and refusals ---

#[test]
fn single_pixels_saturation_and_bad_arguments() {
    let pixel = Padded::from_fn(1, 1, |_, _| 16000u16, 0);
    let gain = Padded::from_fn(1, 1, |_, _| 3.0f32, 0.0);
    let mut out = OwnedImage::new(1, 1);
    let calibration = Calibration {
        dark: None,
        gain: Some(gain.view()),
    };
    correct(&pixel.view(), &calibration, 16383, &mut out.view_mut()).unwrap();
    assert_eq!(pixels(&out), [16383], "clamped to the format");
    let mut summed = OwnedImage::new(1, 1);
    let bright = Padded::from_fn(2, 2, |_, _| 60_000u16, 0);
    bin(
        &bright.view(),
        Binning::new(2, 2).unwrap(),
        BinMode::Sum,
        &mut summed.view_mut(),
    )
    .unwrap();
    assert_eq!(pixels(&summed), [u16::MAX], "a sum saturates");
    assert_eq!(focus(&pixel.view(), FocusMeasure::LaplacianVariance), 0.0);
    assert_eq!(
        focus(
            &Padded::from_fn(4, 4, |_, _| 0u16, 0).view(),
            FocusMeasure::NormalizedVariance
        ),
        0.0
    );

    let wrong = &mut OwnedImage::<u16>::new(2, 1);
    assert!(matches!(
        correct(
            &pixel.view(),
            &Calibration::default(),
            16383,
            &mut wrong.view_mut()
        ),
        Err(KernelError::Size { .. })
    ));
    let float = Padded::from_fn(3, 3, |_, _| 0.5f32, 0.0);
    let larger = &mut OwnedImage::<f32>::new(4, 3);
    assert!(matches!(
        downsample_area(&float.view(), &mut larger.view_mut()),
        Err(KernelError::Parameter(_))
    ));
    let (scratch, blurred) = (&mut OwnedImage::new(3, 3), &mut OwnedImage::new(3, 3));
    for sigma in [-1.0, MAX_SIGMA + 1.0, f32::NAN] {
        let result = gaussian_blur(
            &float.view(),
            sigma,
            &mut scratch.view_mut(),
            &mut blurred.view_mut(),
        );
        assert!(matches!(result, Err(KernelError::Parameter(_))), "{sigma}");
    }
    let mapping = DisplayMapping {
        min: 10,
        max: 10,
        gamma: 1.0,
        colormap: Colormap::Grey,
    };
    assert!(Lut::new(&mapping).is_err());
    assert!(siq_imgproc::histogram(&pixel.view(), 0, &mut Histogram::new(14)).is_err());

    // Inverted grey, and values outside the window.
    let lut = Lut::new(&DisplayMapping {
        min: 100,
        max: 200,
        gamma: 1.0,
        colormap: Colormap::Inverted,
    })
    .unwrap();
    assert_eq!(
        [lut.level(0), lut.level(150), lut.level(65535)],
        [255, 127, 0]
    );
    assert_eq!(lut.colour(0), [255; 3]);
}
