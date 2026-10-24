//! Bands of rows on rayon's pool, split in halves with `rayon::join`, which allocates nothing.
//!
//! The split depends only on the image's size, so a reduction adds its bands in the same order on
//! every run, whatever the threads do.

use std::ops::Range;

use rayon::join;
use siq_types::ImageViewMut;

/// The rows of a band: about 128 Ki pixels, enough work to be worth handing to another thread.
pub(crate) fn band_rows(width: usize) -> usize {
    ((1 << 17) / width.max(1)).max(1)
}

/// Runs `work` on bands of `out`'s rows, in parallel, with each band's first row.
pub(crate) fn for_bands<T: Send>(
    out: ImageViewMut<'_, T>,
    work: &(impl Fn(usize, ImageViewMut<'_, T>) + Sync),
) {
    let rows = band_rows(out.width());
    split(out, 0, rows, work);
}

fn split<T: Send>(
    out: ImageViewMut<'_, T>,
    first: usize,
    rows: usize,
    work: &(impl Fn(usize, ImageViewMut<'_, T>) + Sync),
) {
    if out.height() <= rows {
        return work(first, out);
    }
    let half = out.height() / 2;
    let (top, bottom) = out.split_at_row(half);
    join(
        || split(top, first, rows, work),
        || split(bottom, first + half, rows, work),
    );
}

/// Maps bands of `rows`, at most `per_band` each, in parallel, and combines their results in a
/// fixed order.
pub(crate) fn reduce<R: Send>(
    rows: Range<usize>,
    per_band: usize,
    map: &(impl Fn(Range<usize>) -> R + Sync),
    combine: &(impl Fn(R, R) -> R + Sync),
) -> R {
    if rows.len() <= per_band.max(1) {
        return map(rows);
    }
    let middle = rows.start + rows.len() / 2;
    let (first, second) = join(
        || reduce(rows.start..middle, per_band, map, combine),
        || reduce(middle..rows.end, per_band, map, combine),
    );
    combine(first, second)
}
