//! The canvas's pages (ADR-0059): made where frames fall, kept on the GPU within the budget, and
//! evicted to host RAM, shrunk, least recently used first.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_gpu::lut::Table;
use siq_gpu::wgpu::util::DeviceExt;
use siq_gpu::{Gpu, GpuError, wgpu};
use siq_types::Affine2;

use crate::paint::{self, Frame, Params};
use crate::view::{self, View};

/// A page's side, in canvas pixels.
pub const PAGE: u32 = 4096;
/// A page's bytes: 64 MB.
pub const PAGE_BYTES: u64 = PAGE as u64 * PAGE as u64 * 4;

/// How the canvas keeps its pages, and paints.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CanvasOptions {
    /// The GPU memory its pages may take, in bytes: 2 GB, 32 pages, by default.
    pub budget_bytes: u64,
    /// How far in from a frame's edge its alpha reaches 1, in frame pixels: 64 by default, and
    /// at most an eighth of the frame's shorter side.
    pub feather_px: f32,
    /// How much an evicted page shrinks each way in host RAM: 4 by default.
    pub shrink: u32,
}

impl Default for CanvasOptions {
    fn default() -> Self {
        Self {
            budget_bytes: 2 << 30,
            feather_px: 64.0,
            shrink: 4,
        }
    }
}

/// What a paint did to the pages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Painted {
    /// The pages the frame was painted on.
    pub pages: usize,
    /// Pages made for it.
    pub made: usize,
    /// Pages brought back from host RAM for it.
    pub restored: usize,
    /// Pages evicted to make room.
    pub evicted: usize,
}

enum Place {
    Gpu(wgpu::Buffer),
    /// Shrunk by the options' `shrink`.
    Host(Vec<u32>),
}

struct Page {
    place: Place,
    used: u64,
}

/// The live mosaic's canvas: its pages, by their place in pages. Its methods run on the GPU's
/// thread, in `siq-gpu`'s jobs.
pub struct Canvas {
    options: CanvasOptions,
    pages: HashMap<[i32; 2], Page>,
    tick: u64,
    bounds: Option<[f64; 4]>,
}

impl Canvas {
    /// An empty canvas.
    pub fn new(options: CanvasOptions) -> Self {
        Self {
            options,
            pages: HashMap::new(),
            tick: 0,
            bounds: None,
        }
    }

    /// `frame` painted where `m` (frame pixels to canvas pixels) puts it.
    pub fn paint(
        &mut self,
        gpu: &mut Gpu,
        frame: Frame<'_>,
        m: &Affine2,
    ) -> Result<Painted, GpuError> {
        let footprint = frame.footprint(m);
        let page_of = |at: f64| (at / f64::from(PAGE)).floor() as i32;
        let (columns, rows) = (
            page_of(footprint[0])..=page_of(footprint[2]),
            page_of(footprint[1])..=page_of(footprint[3]),
        );
        let feather = self.options.feather_px;
        let mut covered = Vec::new();
        for row in rows {
            for column in columns.clone() {
                let origin = [
                    i64::from(column) * i64::from(PAGE),
                    i64::from(row) * i64::from(PAGE),
                ];
                if let Some(params) = Params::for_page(&frame, m, origin, PAGE, feather) {
                    covered.push(([column, row], params));
                }
            }
        }
        let keys: Vec<[i32; 2]> = covered.iter().map(|(key, _)| *key).collect();
        let mut painted = Painted {
            pages: covered.len(),
            ..Painted::default()
        };
        if covered.is_empty() {
            return Ok(painted);
        }
        let uploaded = gpu.upload_u16(frame.samples)?;
        for (key, params) in &covered {
            let page = self.resident(gpu, *key, &keys, &mut painted)?;
            paint::gpu(gpu, &uploaded, &page, params)?;
        }
        let [x0, y0, x1, y1] = footprint;
        self.bounds = Some(match self.bounds {
            None => footprint,
            Some([a, b, c, d]) => [a.min(x0), b.min(y0), c.max(x1), d.max(y1)],
        });
        Ok(painted)
    }

    /// Page `key`'s buffer on the GPU: as it is, brought back from host RAM, or made. Room is
    /// made first, evicting the least recently used page that isn't one of `keep`.
    fn resident(
        &mut self,
        gpu: &mut Gpu,
        key: [i32; 2],
        keep: &[[i32; 2]],
        painted: &mut Painted,
    ) -> Result<wgpu::Buffer, GpuError> {
        self.tick += 1;
        if let Some(Page {
            place: Place::Gpu(buffer),
            used,
        }) = self.pages.get_mut(&key)
        {
            *used = self.tick;
            return Ok(buffer.clone());
        }
        let capacity = (self.options.budget_bytes / PAGE_BYTES).max(1) as usize;
        while self.on_gpu() >= capacity {
            let oldest = self
                .pages
                .iter()
                .filter(|(at, page)| matches!(page.place, Place::Gpu(_)) && !keep.contains(at))
                .min_by_key(|(_, page)| page.used)
                .map(|(at, _)| *at);
            let Some(oldest) = oldest else {
                let why = "the VRAM budget holds fewer pages than one frame covers";
                return Err(GpuError::TooLarge(why.into()));
            };
            self.evict(gpu, oldest)?;
            painted.evicted += 1;
        }
        let contents = match self.pages.remove(&key) {
            Some(Page {
                place: Place::Host(shrunk),
                ..
            }) => {
                painted.restored += 1;
                Some(grow(&shrunk, PAGE, self.options.shrink))
            }
            _ => None,
        };
        let usage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST;
        let buffer = match contents {
            Some(pixels) => gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("canvas page"),
                    contents: bytemuck::cast_slice(&pixels),
                    usage,
                }),
            None => {
                painted.made += 1;
                gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("canvas page"),
                    size: PAGE_BYTES,
                    usage,
                    mapped_at_creation: false,
                })
            }
        };
        let page = Page {
            place: Place::Gpu(buffer.clone()),
            used: self.tick,
        };
        self.pages.insert(key, page);
        Ok(buffer)
    }

    /// Page `key` read back and kept in host RAM, shrunk.
    fn evict(&mut self, gpu: &mut Gpu, key: [i32; 2]) -> Result<(), GpuError> {
        let Some(Page {
            place: Place::Gpu(buffer),
            ..
        }) = self.pages.get(&key)
        else {
            return Ok(());
        };
        let bytes = gpu.read(buffer, PAGE_BYTES)?;
        let shrunk = shrink(&words(&bytes), PAGE, self.options.shrink);
        self.pages.get_mut(&key).expect("the page evicted").place = Place::Host(shrunk);
        Ok(())
    }

    /// The canvas's pixels of the rectangle from canvas pixel `origin`, `size` pixels: from host RAM
    /// at its shrunk detail where a page was evicted, and 0 where nothing was painted.
    pub fn region(
        &mut self,
        gpu: &mut Gpu,
        origin: [i64; 2],
        size: [u32; 2],
    ) -> Result<Vec<u32>, GpuError> {
        let side = i64::from(PAGE);
        let [w, h] = size.map(i64::from);
        let mut out = vec![0u32; (w * h) as usize];
        let pages = |from: i64, to: i64| from.div_euclid(side)..=(to - 1).div_euclid(side);
        for row in pages(origin[1], origin[1] + h) {
            for column in pages(origin[0], origin[0] + w) {
                let key = [column as i32, row as i32];
                let Some(page) = self.pages.get(&key) else {
                    continue;
                };
                // The rows of this page in the rectangle, and its columns.
                let (py0, py1) = (
                    (origin[1] - row * side).max(0),
                    (origin[1] + h - row * side).min(side),
                );
                let (px0, px1) = (
                    (origin[0] - column * side).max(0),
                    (origin[0] + w - column * side).min(side),
                );
                let pixel: Box<dyn Fn(i64, i64) -> u32> = match &page.place {
                    Place::Gpu(buffer) => {
                        let offset = (py0 * side * 4) as u64;
                        let bytes = gpu.read_at(buffer, offset, ((py1 - py0) * side * 4) as u64)?;
                        let rows: Vec<u32> = words(&bytes);
                        Box::new(move |x, y| rows[((y - py0) * side + x) as usize])
                    }
                    Place::Host(shrunk) => {
                        let factor = i64::from(self.options.shrink);
                        let across = side / factor;
                        Box::new(move |x, y| shrunk[((y / factor) * across + x / factor) as usize])
                    }
                };
                for y in py0..py1 {
                    for x in px0..px1 {
                        let at = (row * side + y - origin[1]) * w + (column * side + x - origin[0]);
                        out[at as usize] = pixel(x, y);
                    }
                }
            }
        }
        Ok(out)
    }

    /// `view` of the canvas through `table`: a byte a pixel, row by row. A pixel is black where
    /// nothing was painted, darker where little of it is covered, and from host RAM at its shrunk
    /// detail where its page was evicted.
    pub fn render(&self, gpu: &mut Gpu, view: &View, table: &Table) -> Result<Vec<u8>, GpuError> {
        let ([width, height], stride) = (view.size, view.stride());
        let bytes = (u64::from(stride) * u64::from(height) * 4).max(4);
        gpu.fits_binding(bytes, "the view")?;
        let device = &gpu.device;
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("view"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let entries: Vec<u32> = table.iter().map(|&level| u32::from(level)).collect();
        let lut = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("view lut"),
            contents: bytemuck::cast_slice(&entries),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let side = i64::from(PAGE);
        for (key, page) in &self.pages {
            let corner = [i64::from(key[0]) * side, i64::from(key[1]) * side];
            match &page.place {
                Place::Gpu(buffer) => {
                    if let Some(params) = view::Params::for_page(view, corner, PAGE, 1) {
                        view::gpu(gpu, buffer, &lut, &out, &params)?;
                    }
                }
                Place::Host(shrunk) => {
                    let factor = self.options.shrink.max(1);
                    let Some(params) = view::Params::for_page(view, corner, PAGE / factor, factor)
                    else {
                        continue;
                    };
                    let buffer = gpu
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("view shrunk page"),
                            contents: bytemuck::cast_slice(shrunk),
                            usage: wgpu::BufferUsages::STORAGE,
                        });
                    view::gpu(gpu, &buffer, &lut, &out, &params)?;
                }
            }
        }
        let read = gpu.read(&out, bytes)?;
        let rows = read.chunks_exact(stride as usize * 4).take(height as usize);
        Ok(rows
            .flat_map(|row| &row[..width as usize])
            .copied()
            .collect())
    }

    /// Every page gone.
    pub fn clear(&mut self) {
        self.pages.clear();
        self.bounds = None;
    }

    /// The pages on the GPU.
    pub fn on_gpu(&self) -> usize {
        self.pages
            .values()
            .filter(|page| matches!(page.place, Place::Gpu(_)))
            .count()
    }

    /// The pages evicted to host RAM.
    pub fn on_host(&self) -> usize {
        self.pages.len() - self.on_gpu()
    }

    /// The box of everything painted, in canvas pixels: `[x0, y0, x1, y1]`.
    pub fn bounds(&self) -> Option<[f64; 4]> {
        self.bounds
    }
}

/// `page`, `side` pixels a side, shrunk `factor` times each way: each block's value weighted by its
/// coverage, and its coverage averaged.
fn shrink(page: &[u32], side: u32, factor: u32) -> Vec<u32> {
    let (side, factor) = (side as usize, factor.max(1) as usize);
    let across = side / factor;
    let mut out = vec![0u32; across * across];
    for (i, shrunk) in out.iter_mut().enumerate() {
        let (bx, by) = (i % across * factor, i / across * factor);
        let (mut weighted, mut cover) = (0.0f64, 0.0f64);
        for y in by..by + factor {
            for &pixel in &page[y * side + bx..y * side + bx + factor] {
                let c = f64::from(pixel >> 16);
                (weighted, cover) = (weighted + f64::from(pixel & 0xffff) * c, cover + c);
            }
        }
        if cover > 0.0 {
            let value = (weighted / cover).round() as u32;
            *shrunk = value | (((cover / (factor * factor) as f64).round() as u32) << 16);
        }
    }
    out
}

/// `shrunk` grown back to `side` pixels a side: each of its pixels over its block.
fn grow(shrunk: &[u32], side: u32, factor: u32) -> Vec<u32> {
    let (side, factor) = (side as usize, factor.max(1) as usize);
    let across = side / factor;
    (0..side * side)
        .map(|i| shrunk[(i / side / factor) * across + (i % side) / factor])
        .collect()
}

/// Little-endian bytes as 32-bit words.
fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect()
}
