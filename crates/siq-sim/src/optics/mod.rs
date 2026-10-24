//! Image formation (REQ-SIM-001, ADR-0026): a realistic 14-bit frame of a `siq-synth` world, for
//! the true stage pose and focus, through an optics and a sensor model.
//!
//! - [`render_signal`] makes the noiseless signal: the world sampled on the frame's own pixels,
//!   blurred by its defocus, with the vignetting and each pixel's gain.
//! - [`digitize`] adds the noise and the offsets, and quantizes it to the pixel format.
//! - [`form_image`] does both. [`Renderer`] keeps the last signal: while the pose holds, a frame
//!   costs only its noise, and while it moves, a draft is rendered at a lower resolution.

use std::sync::Arc;

use siq_imgproc::KernelError;
use siq_synth::World;
use siq_types::ImageViewMut;

mod models;
pub(crate) mod noise;
mod signal;

pub use models::{OpticsModel, SensorModel, Shot, Vignetting};
pub use noise::digitize;
pub use signal::{Rendered, Scratch, render_signal};

/// Renders frame `seq` of `shot` into `out`, at full detail, through `scratch`.
pub fn form_image(
    world: &dyn World,
    optics: &OpticsModel,
    sensor: &SensorModel,
    shot: &Shot,
    seq: u64,
    scratch: &mut Scratch,
    out: &mut ImageViewMut<'_, u16>,
) -> Result<Rendered, KernelError> {
    let rendered = render_signal(world, optics, sensor, shot, 1, scratch)?;
    digitize(&scratch.signal(), optics, sensor, shot, seq, out)?;
    Ok(rendered)
}

/// Renders the frames of one world, keeping the last signal.
///
/// - The same signal as the last frame: only the noise is new.
/// - A new pose, or a new readout region: a draft, sampled coarsely enough to take about
///   `draft_samples` samples of the world (6 million unless set): a quarter of the resolution at
///   100 MP, and full detail for frames that small.
/// - The same as a draft: the full-detail signal, which is then kept.
pub struct Renderer {
    world: Arc<dyn World>,
    optics: OpticsModel,
    sensor: SensorModel,
    scratch: Scratch,
    /// The last signal's shot, its render, and whether it was asked for at full detail.
    last: Option<(Shot, Rendered, bool)>,
    draft_samples: usize,
}

impl Renderer {
    /// A renderer of `world`, with drafts of about 6 million samples.
    pub fn new(world: Arc<dyn World>, optics: OpticsModel, sensor: SensorModel) -> Self {
        Self {
            world,
            optics,
            sensor,
            scratch: Scratch::default(),
            last: None,
            draft_samples: 6_000_000,
        }
    }

    /// Sets about how many samples of the world a draft takes: `usize::MAX` renders every frame
    /// at full detail.
    pub fn set_draft_samples(&mut self, samples: usize) {
        self.draft_samples = samples.max(1);
    }

    /// The optics.
    pub fn optics(&self) -> &OpticsModel {
        &self.optics
    }

    /// Sets the flux, which only the digitization uses, so the signal is kept.
    pub fn set_flux(&mut self, flux: f64) {
        self.optics.flux = flux;
    }

    /// The optics, to change. The next frame renders a new signal.
    pub fn optics_mut(&mut self) -> &mut OpticsModel {
        self.last = None;
        &mut self.optics
    }

    /// Renders frame `seq` of `shot` into `out`.
    pub fn render(
        &mut self,
        shot: &Shot,
        seq: u64,
        out: &mut ImageViewMut<'_, u16>,
    ) -> Result<Rendered, KernelError> {
        let detail = match self.last {
            Some((last, _, full)) if last.same_signal(shot) => (!full).then_some(1),
            _ => {
                let (width, height) = shot.image_size();
                let ratio = (width * height) as f64 / self.draft_samples as f64;
                Some(ratio.sqrt().ceil().max(1.0) as u32)
            }
        };
        let rendered = match detail {
            Some(detail) => {
                let world = self.world.as_ref();
                let rendered = render_signal(
                    world,
                    &self.optics,
                    &self.sensor,
                    shot,
                    detail,
                    &mut self.scratch,
                )?;
                self.last = Some((*shot, rendered, detail == 1));
                rendered
            }
            None => {
                let (_, rendered, _) = self.last.expect("a cached signal");
                Rendered {
                    cached: true,
                    ..rendered
                }
            }
        };
        digitize(
            &self.scratch.signal(),
            &self.optics,
            &self.sensor,
            shot,
            seq,
            out,
        )?;
        Ok(rendered)
    }
}
