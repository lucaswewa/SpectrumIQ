//! A dark frame as a NumPy `.npy` file (format 1.0, little-endian `u16`, C order), so that
//! `numpy.load` reads it as it is.

use siq_types::{ImageView, OwnedImage};

const MAGIC: &[u8] = b"\x93NUMPY\x01\x00";

/// `image` as an `.npy` file's bytes.
pub(crate) fn write(image: &ImageView<'_, u16>) -> Vec<u8> {
    let (width, height) = (image.width(), image.height());
    let mut header =
        format!("{{'descr': '<u2', 'fortran_order': False, 'shape': ({height}, {width}), }}");
    // The data starts on a multiple of 64 bytes; the header ends with a newline.
    let unpadded = MAGIC.len() + 2 + header.len() + 1;
    header.push_str(&" ".repeat(unpadded.next_multiple_of(64) - unpadded));
    header.push('\n');
    let mut bytes = Vec::with_capacity(MAGIC.len() + 2 + header.len() + width * height * 2);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
    bytes.extend_from_slice(header.as_bytes());
    for value in image.rows().flatten() {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// The image of an `.npy` file's `bytes`, as [`write`] makes them.
pub(crate) fn read(bytes: &[u8]) -> Result<OwnedImage<u16>, String> {
    let length = match bytes.strip_prefix(MAGIC) {
        Some([low, high, ..]) => usize::from(u16::from_le_bytes([*low, *high])),
        _ => return Err("not an .npy file of format 1.0".to_owned()),
    };
    let start = MAGIC.len() + 2;
    let header = bytes
        .get(start..start + length)
        .ok_or("a truncated header")?;
    let header = String::from_utf8_lossy(header);
    let shape = header
        .split_once("'shape': (")
        .and_then(|(_, rest)| rest.split_once(')'));
    let mut sizes = shape
        .ok_or("no shape")?
        .0
        .split(',')
        .map(|n| n.trim().parse::<usize>());
    let (Some(Ok(height)), Some(Ok(width))) = (sizes.next(), sizes.next()) else {
        return Err("not two-dimensional".to_owned());
    };
    let data = &bytes[start + length..];
    let ours = header.contains("'descr': '<u2', 'fortran_order': False");
    if !ours || data.len() != width * height * 2 {
        return Err("not C-ordered little-endian u16 of its shape".to_owned());
    }
    Ok(OwnedImage::from_fn(width, height, |x, y| {
        let at = (y * width + x) * 2;
        u16::from_le_bytes([data[at], data[at + 1]])
    }))
}
