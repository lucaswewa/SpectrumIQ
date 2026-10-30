//! The colour filter kernels: demosaicing at size and shrunk against naive references, exact on
//! linear colours inside the image, and each colour's histogram, white and mapping.

use proptest::prelude::*;
use siq_imgproc::*;
use siq_types::{Cfa, ImageView, ImageViewMut, OwnedImage};

const CFAS: [Cfa; 4] = [Cfa::Rggb, Cfa::Grbg, Cfa::Gbrg, Cfa::Bggr];

fn cfa() -> impl Strategy<Value = Cfa> {
    prop::sample::select(CFAS.to_vec())
}

/// A mosaic of `width` × `height`, its rows 3 pixels apart, of `pixel`'s colours through `cfa`.
fn mosaic(
    cfa: Cfa,
    width: usize,
    height: usize,
    pixel: impl Fn(usize, usize) -> [u16; 3],
) -> (Vec<u16>, usize) {
    let stride = width + 3;
    let data = (0..stride * height)
        .map(|i| match (i % stride, i / stride) {
            (x, y) if x < width => pixel(x, y)[cfa.colour(x, y)],
            _ => u16::MAX,
        })
        .collect();
    (data, stride)
}

fn demosaiced(
    cfa: Cfa,
    (data, stride): &(Vec<u16>, usize),
    (width, height): (usize, usize),
    (w, h): (usize, usize),
) -> Vec<[u16; 3]> {
    let input = ImageView::new(data, width, height, *stride).unwrap();
    let mut out = vec![[0; 3]; w * h];
    demosaic(
        &input,
        cfa,
        &mut ImageViewMut::new(&mut out, w, h, w).unwrap(),
    )
    .unwrap();
    out
}

/// Each pixel's own colour, and for each other one the rounded mean of the 3 × 3 neighbours of
/// that colour, reflected at the edges.
fn interpolated(
    cfa: Cfa,
    at: impl Fn(usize, usize) -> u16,
    width: usize,
    height: usize,
) -> Vec<[u16; 3]> {
    let reflect = |i: isize, n: usize| {
        let n = n as isize;
        let i = if i < 0 {
            -i
        } else if i >= n {
            2 * n - 2 - i
        } else {
            i
        };
        i.clamp(0, n - 1) as usize
    };
    let mut out = Vec::new();
    for y in 0..height {
        for x in 0..width {
            out.push(std::array::from_fn(|c| {
                if c == cfa.colour(x, y) {
                    return at(x, y);
                }
                let mut seen = Vec::new();
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (sx, sy) = (
                            reflect(x as isize + dx, width),
                            reflect(y as isize + dy, height),
                        );
                        if cfa.colour(sx, sy) == c {
                            seen.push(u32::from(at(sx, sy)));
                        }
                    }
                }
                match seen.len() as u32 {
                    0 => at(x, y),
                    n => ((seen.iter().sum::<u32>() + n / 2) / n) as u16,
                }
            }));
        }
    }
    out
}

/// Each output pixel's mean of each colour over the whole cells it covers.
fn from_cells(
    cfa: Cfa,
    at: impl Fn(usize, usize) -> u16,
    (width, height): (usize, usize),
    (w, h): (usize, usize),
) -> Vec<[u16; 3]> {
    let span = |i: usize, n: usize, cells: usize| {
        let start = i * cells / n;
        start..((i + 1) * cells / n).max(start + 1)
    };
    let mut out = Vec::new();
    for oy in 0..h {
        for ox in 0..w {
            let (mut sums, mut counts) = ([0u64; 3], [0u64; 3]);
            for y in 2 * span(oy, h, height / 2).start..2 * span(oy, h, height / 2).end {
                for x in 2 * span(ox, w, width / 2).start..2 * span(ox, w, width / 2).end {
                    sums[cfa.colour(x, y)] += u64::from(at(x, y));
                    counts[cfa.colour(x, y)] += 1;
                }
            }
            out.push(std::array::from_fn(|c| {
                ((sums[c] + counts[c] / 2) / counts[c]) as u16
            }));
        }
    }
    out
}

proptest! {
    #[test]
    fn at_its_size_a_mosaic_interpolates_as_the_reference(
        cfa in cfa(),
        (width, height) in (1..30usize, 1..20usize),
        seed in any::<u64>(),
    ) {
        let value = |x: usize, y: usize| ((seed ^ (x as u64 * 7919 + y as u64 * 104_729)).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 50) as u16;
        let input = mosaic(cfa, width, height, |x, y| [value(x, y); 3]);
        let found = demosaiced(cfa, &input, (width, height), (width, height));
        prop_assert_eq!(found, interpolated(cfa, value, width, height));
    }

    #[test]
    fn shrunk_a_mosaic_is_its_cells_means(
        cfa in cfa(),
        (width, height) in (2..40usize, 2..30usize),
        (w, h) in (1..40usize, 1..30usize),
        seed in any::<u64>(),
    ) {
        prop_assume!(w <= width && h <= height && (w, h) != (width, height));
        let value = |x: usize, y: usize| ((seed ^ (x as u64 * 31 + y as u64 * 1009)).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 50) as u16;
        let input = mosaic(cfa, width, height, |x, y| [value(x, y); 3]);
        let found = demosaiced(cfa, &input, (width, height), (w, h));
        prop_assert_eq!(found, from_cells(cfa, value, (width, height), (w, h)));
    }
}

#[test]
fn inside_the_image_linear_colours_interpolate_exactly() {
    // Each colour a plane of its own; a large image, which splits into bands.
    let colour = |x: usize, y: usize| {
        let (x, y) = (x as u16, y as u16);
        [1000 + 3 * x + 2 * y, 4000 + x + 5 * y, 2000 + 2 * x + y]
    };
    for cfa in CFAS {
        let (width, height) = (700, 400);
        let input = mosaic(cfa, width, height, colour);
        let found = demosaiced(cfa, &input, (width, height), (width, height));
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                assert_eq!(found[y * width + x], colour(x, y), "{cfa:?} at ({x}, {y})");
            }
        }
    }
}

#[test]
fn a_mosaic_of_one_colour_shows_that_colour_at_any_size() {
    for cfa in CFAS {
        let input = mosaic(cfa, 600, 500, |_, _| [3000, 4000, 2000]);
        for size in [(600, 500), (300, 250), (97, 61), (1, 1)] {
            let found = demosaiced(cfa, &input, (600, 500), size);
            assert!(
                found.iter().all(|&p| p == [3000, 4000, 2000]),
                "{cfa:?} at {size:?}"
            );
        }
    }
}

#[test]
fn a_mosaic_shows_at_most_at_its_size() {
    let input = OwnedImage::<u16>::new(4, 4);
    let mut out = vec![[0; 3]; 25];
    let mut larger = ImageViewMut::new(&mut out, 5, 5, 5).unwrap();
    assert!(demosaic(&input.view(), Cfa::Rggb, &mut larger).is_err());
    let line = OwnedImage::<u16>::new(5, 1);
    let mut out = vec![[0; 3]; 2];
    let mut shrunk = ImageViewMut::new(&mut out, 2, 1, 2).unwrap();
    assert!(
        demosaic(&line.view(), Cfa::Rggb, &mut shrunk).is_err(),
        "no whole cell"
    );
}

#[test]
fn each_colour_s_histogram_counts_its_samples() {
    let (data, stride) = mosaic(Cfa::Grbg, 101, 51, |_, _| [3000, 4000, 2000]);
    let input = ImageView::new(&data, 101, 51, stride).unwrap();
    let mut counts = std::array::from_fn(|_| Histogram::new(14));
    histogram_cfa(&input, Cfa::Grbg, 1, &mut counts).unwrap();
    // 50 × 25 whole cells, and the odd row and column's: red at odd columns of even rows.
    let red = 50 * 26;
    let blue = 51 * 25;
    let green = 101 * 51 - red - blue;
    let found = counts.each_ref().map(|c| (c.count(), c.percentile(0.5)));
    assert_eq!(found, [(red, 3000), (green, 4000), (blue, 2000)]);
    // Every second cell both ways: about a quarter.
    let mut sparse = std::array::from_fn(|_| Histogram::new(14));
    histogram_cfa(&input, Cfa::Grbg, 2, &mut sparse).unwrap();
    assert_eq!(sparse[0].count(), 25 * 13);
}

#[test]
fn white_balance_brings_each_colour_s_brightest_to_green_s_and_maps_through_it() {
    let (data, stride) = mosaic(Cfa::Rggb, 64, 64, |x, _| {
        [1500 + x as u16, 2000 + x as u16, 1000]
    });
    let input = ImageView::new(&data, 64, 64, stride).unwrap();
    let mut counts = std::array::from_fn(|_| Histogram::new(16));
    histogram_cfa(&input, Cfa::Rggb, 1, &mut counts).unwrap();
    let gains = white_gains(&counts, 1.0);
    let expected = [2063.0 / 1562.0, 1.0, 2.063];
    for (gain, expected) in gains.into_iter().zip(expected) {
        assert!((gain - expected).abs() < 0.01, "{gains:?}");
    }

    let mapping = DisplayMapping {
        min: 100,
        max: 4000,
        gamma: 1.0,
        colormap: Colormap::Colour([0, 255, 0]),
    };
    let [red, green, blue] = balanced(&mapping, [2.0, 1.0, 0.0]);
    assert_eq!((red.min, red.max, red.colormap), (50, 2000, Colormap::Grey));
    assert_eq!((green.min, green.max), (100, 4000));
    assert_eq!((blue.min, blue.max), (100, 4000), "a gain of 0 is none");
    let inverted = balanced(
        &DisplayMapping {
            colormap: Colormap::Inverted,
            ..mapping
        },
        [1.0; 3],
    );
    assert!(inverted.iter().all(|m| m.colormap == Colormap::Inverted));

    let luts = [red, green, blue].map(|m| Lut::new(&m).unwrap());
    let rgb = OwnedImage::from_fn(3, 1, |x, _| {
        [[2000, 4000, 4000], [50, 100, 100], [1025, 2050, 2050]][x]
    });
    let mut shown = OwnedImage::<[u8; 3]>::new(3, 1);
    map_colours(&luts, &rgb.view(), &mut shown.view_mut()).unwrap();
    let shown: Vec<_> = shown.view().rows().flatten().copied().collect();
    assert_eq!(shown, [[255; 3], [0; 3], [128; 3]]);
}
