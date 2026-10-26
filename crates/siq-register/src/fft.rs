//! 2D FFTs of real images of any size: each row by `realfft` (real to half-complex), then each
//! column of the half spectrum by `rustfft`. The inverse runs the other way. Neither direction is
//! normalised: phase correlation only needs where the peak is, and how it stands out.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use rustfft::{Fft, FftPlanner};

/// The plans for images of one size.
pub(crate) struct Plans {
    width: usize,
    height: usize,
    rows: Arc<dyn RealToComplex<f32>>,
    rows_back: Arc<dyn ComplexToReal<f32>>,
    columns: Arc<dyn Fft<f32>>,
    columns_back: Arc<dyn Fft<f32>>,
}

impl Plans {
    pub(crate) fn new(width: usize, height: usize) -> Self {
        let (mut real, mut complex) = (RealFftPlanner::new(), FftPlanner::new());
        Self {
            width,
            height,
            rows: real.plan_fft_forward(width),
            rows_back: real.plan_fft_inverse(width),
            columns: complex.plan_fft_forward(height),
            columns_back: complex.plan_fft_inverse(height),
        }
    }

    /// The number of complex values in a row of the half spectrum.
    fn half(&self) -> usize {
        self.width / 2 + 1
    }

    /// The spectrum of `image`, `height` rows of `width`, which it uses as scratch: `height` rows
    /// of `width / 2 + 1`.
    pub(crate) fn forward(&self, image: &mut [f32]) -> Vec<Complex<f32>> {
        let half = self.half();
        let mut spectrum = vec![Complex::default(); half * self.height];
        for (row, out) in image
            .chunks_exact_mut(self.width)
            .zip(spectrum.chunks_exact_mut(half))
        {
            self.rows.process(row, out).expect("the row fits its plan");
        }
        self.each_column(&mut spectrum, self.columns.as_ref());
        spectrum
    }

    /// The image of `spectrum`, which it uses as scratch.
    pub(crate) fn inverse(&self, spectrum: &mut [Complex<f32>]) -> Vec<f32> {
        let half = self.half();
        self.each_column(spectrum, self.columns_back.as_ref());
        let mut image = vec![0.0; self.width * self.height];
        for (row, out) in spectrum
            .chunks_exact_mut(half)
            .zip(image.chunks_exact_mut(self.width))
        {
            // A real row's first value, and its last for an even width, are real.
            row[0].im = 0.0;
            if self.width.is_multiple_of(2) {
                row[half - 1].im = 0.0;
            }
            self.rows_back
                .process(row, out)
                .expect("the row fits its plan");
        }
        image
    }

    fn each_column(&self, spectrum: &mut [Complex<f32>], fft: &dyn Fft<f32>) {
        let half = self.half();
        let mut column = vec![Complex::default(); self.height];
        for k in 0..half {
            for (y, value) in column.iter_mut().enumerate() {
                *value = spectrum[y * half + k];
            }
            fft.process(&mut column);
            for (y, value) in column.iter().enumerate() {
                spectrum[y * half + k] = *value;
            }
        }
    }
}
