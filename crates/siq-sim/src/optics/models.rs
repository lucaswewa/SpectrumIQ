//! The optics and the sensor of the simulated camera, and the shot: what one frame is of.

use siq_devices::CameraSettings;
use siq_types::{
    Affine2, Cfa, Micrometers, Nanometers, PixelToStage, PointPx, StagePose, StageToPixel,
};

/// How light falls off toward the corners of the sensor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Vignetting {
    /// None: a flat field.
    None,
    /// The cos⁴ law, scaled so that the sensor's corners get `corner` of the centre's light.
    Cos4 {
        /// The corners' share of the centre's light, from 0 to 1.
        corner: f64,
    },
    /// `1 + a ρ² + b ρ⁴`, where ρ is the distance from the centre over the half-diagonal.
    Polynomial {
        /// The coefficient of ρ².
        a: f64,
        /// The coefficient of ρ⁴.
        b: f64,
    },
}

impl Vignetting {
    /// The illumination at `rho2`, the squared distance from the centre over the half-diagonal,
    /// relative to the centre's.
    pub fn at(&self, rho2: f64) -> f64 {
        self.profile().at(rho2)
    }

    /// The profile with its constants worked out, for a pass over a frame.
    pub(crate) fn profile(&self) -> Profile {
        match *self {
            Self::None => Profile {
                tan2: 0.0,
                a: 0.0,
                b: 0.0,
            },
            // cos⁴θ = 1 / (1 + tan²θ)², with tan θ growing with ρ to its value at the corners.
            Self::Cos4 { corner } => Profile {
                tan2: corner.clamp(1e-6, 1.0).powf(-0.5) - 1.0,
                a: 0.0,
                b: 0.0,
            },
            Self::Polynomial { a, b } => Profile { tan2: 0.0, a, b },
        }
    }
}

/// A vignetting profile, `(1 + a ρ² + b ρ⁴) / (1 + tan² ρ²)²`, which covers every kind.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Profile {
    tan2: f64,
    a: f64,
    b: f64,
}

impl Profile {
    pub(crate) fn at(&self, rho2: f64) -> f64 {
        (1.0 + self.a * rho2 + self.b * rho2 * rho2).max(0.0) / (1.0 + rho2 * self.tan2).powi(2)
    }
}

/// The objective and the illumination.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpticsModel {
    /// The objective's numerical aperture.
    pub numerical_aperture: f64,
    /// The refractive index between the objective and the sample: 1 for air.
    pub immersion_index: f64,
    /// The light's wavelength.
    pub wavelength: Nanometers,
    /// The fall-off toward the corners.
    pub vignetting: Vignetting,
    /// The electrons an unbinned sensor pixel collects per millisecond where the world's
    /// intensity is 1.
    pub flux: f64,
}

impl Default for OpticsModel {
    /// A 10×/0.30 dry objective in green light, 20 % darker in the corners, filling about two
    /// thirds of the default sensor's well in 10 ms.
    fn default() -> Self {
        Self {
            numerical_aperture: 0.30,
            immersion_index: 1.0,
            wavelength: Nanometers(550.0),
            vignetting: Vignetting::Cos4 { corner: 0.8 },
            flux: 2000.0,
        }
    }
}

impl OpticsModel {
    /// The blur's standard deviation on the sample at `defocus`. The diffraction-limited spot,
    /// 0.21 λ / NA, and half the radius of the geometric cone of half-angle asin(NA / n) add in
    /// quadrature.
    pub fn blur(&self, defocus: Micrometers) -> Micrometers {
        let aperture = (self.numerical_aperture / self.immersion_index).clamp(0.0, 0.999);
        let diffraction = 0.21 * self.wavelength.0 * 1e-3 / self.numerical_aperture;
        let geometric = 0.5 * defocus.0.abs() * aperture.asin().tan();
        Micrometers(diffraction.hypot(geometric))
    }
}

/// The sensor: its noise, its fixed pattern, and a colour sensor's filters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SensorModel {
    /// The seed of the fixed pattern: the same seed, the same sensor.
    pub seed: u64,
    /// The electrons per ADU at a gain of 1.
    pub electrons_per_adu: f64,
    /// The read noise of one pixel, in electrons RMS.
    pub read_noise: f64,
    /// The dark offset, in ADU.
    pub dark_offset: f64,
    /// The spread of each pixel's own offset, in ADU RMS: dark-signal non-uniformity.
    pub dsnu: f64,
    /// The spread of each pixel's own gain, relative RMS: photo-response non-uniformity.
    pub prnu: f64,
    /// A colour sensor's filter array at its top left; `None` for a mono sensor.
    pub cfa: Option<Cfa>,
    /// Behind its filter, red's, green's and blue's share of the light: the lamp's colour.
    pub response: [f64; 3],
}

impl Default for SensorModel {
    /// A 14-bit sensor of about 32 000 e⁻ full well at 2 e⁻/ADU, with 2.5 e⁻ read noise, an
    /// offset of 100 ADU, 1 ADU of offset spread and 0.5 % of gain spread; mono, or if given a
    /// filter array, under a warm lamp, red at 75 % and blue at 55 % of green.
    fn default() -> Self {
        Self {
            seed: 0,
            electrons_per_adu: 2.0,
            read_noise: 2.5,
            dark_offset: 100.0,
            dsnu: 1.0,
            prnu: 0.005,
            cfa: None,
            response: [0.75, 1.0, 0.55],
        }
    }
}

/// One frame to render: where the stage and focus are, how the camera sees the sample, and how
/// it is set up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    /// The true pose.
    pub pose: StagePose,
    /// The linear map from stage µm to unbinned sensor pixels (ADR-0018).
    pub calibration: Affine2,
    /// The whole sensor's width and height. Its centre is the optical axis.
    pub sensor: (u32, u32),
    /// The ROI, binning, format, exposure and gain.
    pub settings: CameraSettings,
}

impl Shot {
    /// The mapping from the frame's image pixels to the stage.
    pub fn to_stage(&self) -> PixelToStage {
        let centre = PointPx::new(
            f64::from(self.sensor.0) / 2.0,
            f64::from(self.sensor.1) / 2.0,
        );
        let (roi, binning) = (self.settings.roi, self.settings.binning);
        StageToPixel::for_frame(&self.calibration, self.pose.xy(), centre, roi, binning)
            .inverse()
            .expect("a calibration maps the plane onto the sensor")
    }

    /// The frame's width and height, after binning.
    pub fn image_size(&self) -> (usize, usize) {
        let (width, height) = self.settings.image_size();
        (width as usize, height as usize)
    }

    /// Whether `other` sees the same signal: the same pose, mapping and readout region. The
    /// exposure, gain and format change only the digitization.
    pub fn same_signal(&self, other: &Self) -> bool {
        let (a, b) = (&self.settings, &other.settings);
        (self.pose, self.calibration, self.sensor) == (other.pose, other.calibration, other.sensor)
            && (a.roi, a.binning) == (b.roi, b.binning)
    }
}
