use super::*;

#[test]
fn a_view_checks_its_layout() {
    let data = [0u16; 10];
    assert_eq!(
        ImageView::new(&data, 3, 3, 4).unwrap_err(),
        ImageError::BufferTooSmall {
            needed: 11,
            len: 10
        }
    );
    assert_eq!(
        ImageView::new(&data, 4, 2, 3).unwrap_err(),
        ImageError::StrideTooSmall {
            stride: 3,
            width: 4
        }
    );
    assert!(ImageView::new(&data, 3, 3, 3).is_ok());
    // The last row needs no padding after it.
    assert!(ImageView::new(&data[..9], 3, 2, 6).is_ok());
    let empty = ImageView::new(&data[..0], 0, 5, 0).unwrap();
    assert_eq!(empty.rows().count(), 5);
    assert!(ImageView::new(&data, usize::MAX, 3, usize::MAX).is_err());
}

#[test]
fn a_view_without_columns_needs_no_data() {
    // A zero-width view with a stride: its rows would start past the end of the data.
    let view = ImageView::<u16>::new(&[], 0, 3, 8).unwrap();
    assert!(view.rows().all(<[u16]>::is_empty));
    let image = OwnedImage::<u16>::from_fn(4, 4, |x, _| x as u16);
    let column = image.view().sub_view(RectPx::new(4, 1, 0, 3)).unwrap();
    assert_eq!(column.rows().count(), 3);
    assert!(column.rows().all(<[u16]>::is_empty));
}

#[test]
fn rows_skip_the_padding_between_them() {
    let data: Vec<u16> = (0..11).collect();
    let view = ImageView::new(&data, 3, 3, 4).unwrap();
    let rows: Vec<&[u16]> = view.rows().collect();
    assert_eq!(rows, [&[0, 1, 2][..], &[4, 5, 6], &[8, 9, 10]]);
    assert_eq!(view.get(2, 1), Some(&6));
    assert_eq!(view.get(3, 0), None);
    assert_eq!(view.get(0, 3), None);
    assert_eq!((view.width(), view.height(), view.stride()), (3, 3, 4));
}

#[test]
#[should_panic(expected = "row 3 of a 3-row image")]
fn a_row_outside_the_image_panics() {
    let data = [0u8; 9];
    ImageView::new(&data, 3, 3, 3).unwrap().row(3);
}

#[test]
fn sub_views_share_memory_and_nest() {
    let image = OwnedImage::<u16>::from_fn(6, 5, |x, y| (x + 10 * y) as u16);
    let view = image.view();
    let sub = view.sub_view(RectPx::new(1, 2, 3, 2)).unwrap();
    assert_eq!(sub.row(0), [21, 22, 23]);
    assert_eq!(sub.row(1), [31, 32, 33]);
    assert_eq!(sub.row(0).as_ptr(), view.row(2)[1..].as_ptr());
    let nested = sub.sub_view(RectPx::new(1, 1, 2, 1)).unwrap();
    assert_eq!(nested.row(0), [32, 33]);
    assert!(matches!(
        view.sub_view(RectPx::new(4, 0, 3, 1)),
        Err(ImageError::OutOfBounds { .. })
    ));
    // An empty rectangle at the far corner is still inside.
    let corner = view.sub_view(RectPx::new(6, 5, 0, 0)).unwrap();
    assert_eq!((corner.width(), corner.height()), (0, 0));
}

#[test]
fn mutable_sub_views_change_the_parent() {
    let mut image = OwnedImage::<u16>::new(4, 3);
    {
        let mut view = image.view_mut();
        let mut sub = view.sub_view_mut(RectPx::new(1, 1, 2, 2)).unwrap();
        sub.fill(7);
        sub.row_mut(1)[0] = 9;
        assert_eq!(sub.as_view().row(0), [7, 7]);
    }
    let rows: Vec<&[u16]> = image.view().rows().collect();
    assert_eq!(rows, [&[0, 0, 0, 0][..], &[0, 7, 7, 0], &[0, 9, 7, 0]]);
}

#[test]
fn rows_mut_yields_each_row_once() {
    let mut image = OwnedImage::<u8>::new(5, 4);
    let mut view = image.view_mut();
    let mut count = 0;
    for (y, row) in view.rows_mut().enumerate() {
        assert_eq!(row.len(), 5);
        row.fill(y as u8);
        count += 1;
    }
    assert_eq!(count, 4);
    assert_eq!(image.view().row(3), [3; 5]);
}

#[test]
fn split_rows_cover_the_image_once_even_without_trailing_padding() {
    // 3 rows of 4 in a stride of 6, with no padding after the last row.
    let mut data = [0u8; 16];
    let view = ImageViewMut::new(&mut data, 4, 3, 6).unwrap();
    let (mut top, bottom) = view.split_at_row(1);
    let (mut middle, mut last) = bottom.split_at_row(1);
    assert_eq!((top.height(), middle.height(), last.height()), (1, 1, 1));
    top.fill(1);
    middle.fill(2);
    last.fill(3);
    let (whole, empty) = ImageViewMut::new(&mut data, 4, 3, 6)
        .unwrap()
        .split_at_row(9);
    assert_eq!((whole.height(), empty.height(), empty.width()), (3, 0, 4));
    assert_eq!(data, [1, 1, 1, 1, 0, 0, 2, 2, 2, 2, 0, 0, 3, 3, 3, 3]);
}

#[test]
fn owned_rows_start_on_the_row_alignment() {
    for (width, height) in [(1, 1), (31, 3), (100, 7)] {
        let image = OwnedImage::<u16>::new(width, height);
        assert_eq!(image.stride() * 2 % ROW_ALIGNMENT, 0);
        for row in image.view().rows() {
            assert_eq!(row.as_ptr().addr() % ROW_ALIGNMENT, 0);
        }
        let floats = OwnedImage::<f32>::new(width, height);
        assert!(
            floats
                .view()
                .rows()
                .all(|row| row.as_ptr().addr() % ROW_ALIGNMENT == 0)
        );
    }
}

#[test]
fn a_clone_keeps_the_pixels_and_the_alignment() {
    let image = OwnedImage::<u16>::from_fn(9, 4, |x, y| (x * y) as u16);
    let clone = image.clone();
    assert!(clone.view().rows().eq(image.view().rows()));
    assert!(
        clone
            .view()
            .rows()
            .all(|row| row.as_ptr().addr() % ROW_ALIGNMENT == 0)
    );
}

#[test]
fn pixel_formats_say_their_depth() {
    assert_eq!(
        (
            PixelFormat::Mono8.max_value(),
            PixelFormat::Mono8.bytes_per_pixel()
        ),
        (255, 1)
    );
    assert_eq!(
        (PixelFormat::Mono14.max_value(), PixelFormat::Mono14.bits()),
        (16383, 14)
    );
    assert_eq!(PixelFormat::Mono16.max_value(), u16::MAX);
    assert_eq!(PixelFormat::Mono10.bytes_per_pixel(), 2);
    assert_eq!(
        serde_json::to_string(&PixelFormat::Mono14).unwrap(),
        "\"Mono14\""
    );
}

#[test]
fn binning_divides_the_region_and_is_never_zero() {
    assert_eq!(Binning::new(0, 1), None);
    let binning = Binning::new(2, 4).unwrap();
    assert_eq!(binning.factors(), (2, 4));
    assert_eq!(binning.image_size(1001, 803), (500, 200));
    assert_eq!(Binning::default().factors(), (1, 1));
    assert!(serde_json::from_str::<Binning>(r#"{"horizontal":0,"vertical":1}"#).is_err());
    assert_eq!(
        RectPx::from(Roi::full(640, 480)),
        RectPx::new(0, 0, 640, 480)
    );
}

// REQ-ACQ-001: a region that starts an odd column or row in sees the sensor's filter cell
// shifted, and an even one sees it unchanged.
#[test]
fn an_odd_offset_shifts_the_colour_filter_cell() {
    use Cfa::{Bggr, Gbrg, Grbg, Rggb};
    for cfa in [Rggb, Grbg, Gbrg, Bggr] {
        assert_eq!(cfa.shifted(0, 0), cfa);
        assert_eq!(cfa.shifted(1024, 512), cfa);
        assert_eq!(cfa.shifted(1, 1).shifted(1, 1), cfa);
    }
    assert_eq!(
        [Rggb.shifted(1, 0), Rggb.shifted(0, 1), Rggb.shifted(1, 1)],
        [Grbg, Gbrg, Bggr]
    );
    // The Adimec's BayerGB, one column and one row in: green and red, then blue and green.
    assert_eq!(Gbrg.shifted(1, 1), Grbg);
}

#[test]
fn each_pixel_has_the_colour_its_cell_names_and_a_shift_keeps_it() {
    use Cfa::{Bggr, Gbrg, Grbg, Rggb};
    // Red 0, green 1, blue 2, across the cell's top row, then its bottom row.
    let cells = [
        (Rggb, [0, 1, 1, 2]),
        (Grbg, [1, 0, 2, 1]),
        (Gbrg, [1, 2, 0, 1]),
        (Bggr, [2, 1, 1, 0]),
    ];
    for (cfa, colours) in cells {
        let cell = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(x, y)| cfa.colour(x, y));
        assert_eq!(cell, colours, "{cfa:?}");
        for (x, y) in [(3, 0), (0, 5), (7, 9)] {
            let at = cfa.colour(x, y);
            assert_eq!(cfa.colour(x + 2, y + 4), at, "the pattern repeats");
            let shifted = cfa.shifted(x as u32, y as u32);
            assert_eq!(shifted.colour(0, 0), at, "{cfa:?} from ({x}, {y})");
        }
    }
}
