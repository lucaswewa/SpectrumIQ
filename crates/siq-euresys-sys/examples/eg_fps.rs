//! Measures the frame rate a Coaxlink camera reaches under software trigger, on its raw data.
//!
//! ```text
//! cargo run --release -p siq-euresys-sys --features sdk --example eg_fps -- [options] [folder]
//!
//!   --seconds S       how long to measure, 10 by default
//!   --exposure-us E   the exposure, in µs; the camera's own by default
//!   --depth N         software triggers kept in flight, 2 by default (1: one frame at a time)
//!   --overlap         let the next exposure start during the previous readout
//!   --camera I        the camera, by discovery index, 0 by default
//!   --save FOLDER     write the last frame there, as a 16-bit TIFF of the raw mosaic (also as
//!                     a bare argument)
//! ```
//!
//! For the length of the test, it:
//!
//! - sets the camera to its deepest pixel format of the same kind: `BayerGB12` for `BayerGB8`;
//! - triggers the camera over CoaXPress (`ExposureMode = TimedTriggerControl`,
//!   `TriggerSource = Trigger`). The Coaxlink sends a trigger at each `StartCycle` that software
//!   executes (`CameraControlMethod = RC`, `CycleTriggerSource = StartCycle`).
//!   - Up to `--depth` triggers are in flight: the grabber queues them, and starts each cycle once
//!     the previous readout ends, or during it with `--overlap`.
//! - receives the frames, unpacked to 16 bits (`UnpackingMode = Lsb`) but not debayered, into
//!   page-aligned memory of its own.
//!
//! It prints the rate by the grabber's clock and by the wall clock, the latency from trigger to
//! frame, and every loss it can see. It puts back each setting it changed.
//!
//! The feature names are those of the Adimec D-103A12c on a Coaxlink Quad CXP-12. Another camera
//! may name its trigger differently.

#![allow(unsafe_code)]

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_void};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::ptr;
use std::time::{Duration, Instant};

use siq_euresys_sys::*;

type Result<T> = std::result::Result<T, String>;

fn check(status: SiqEgStatus, what: &str) -> Result<()> {
    match status {
        SIQ_EG_OK => Ok(()),
        _ => Err(format!("{what}: {}, {}", status_name(status), last_error())),
    }
}

fn c(text: &str) -> Result<CString> {
    CString::new(text).map_err(|error| error.to_string())
}

/// A string from the shim: its size first, then its text.
fn read(
    what: &str,
    call: impl Fn(*mut c_char, usize, *mut usize) -> SiqEgStatus,
) -> Result<String> {
    let mut needed = 0;
    let status = call(ptr::null_mut(), 0, &mut needed);
    if status != SIQ_EG_TRUNCATED {
        check(status, what)?;
    }
    let mut buf = vec![0u8; needed.max(1)];
    check(
        call(buf.as_mut_ptr().cast(), buf.len(), ptr::null_mut()),
        what,
    )?;
    let text = CStr::from_bytes_until_nul(&buf).map_err(|error| error.to_string())?;
    Ok(text.to_string_lossy().into_owned())
}

/// A GenTL producer, and what it discovered.
struct System(*mut SiqEgSystem);

impl System {
    /// The default producer, with its cameras counted.
    fn open() -> Result<(Self, i32)> {
        let mut raw = ptr::null_mut();
        // SAFETY: a null producer is the default one, and `raw` is writable.
        check(
            unsafe { siq_eg_system_open(ptr::null(), &mut raw) },
            "loading the producer",
        )?;
        let system = Self(raw);
        let mut counts = SiqEgCounts {
            interfaces: 0,
            grabbers: 0,
            cameras: 0,
        };
        // SAFETY: the system is open, and `counts` is writable.
        check(unsafe { siq_eg_discover(raw, &mut counts) }, "discovery")?;
        Ok((system, counts.cameras))
    }
}

impl Drop for System {
    fn drop(&mut self) {
        // SAFETY: open; its grabber borrows it, so it was closed first.
        unsafe { siq_eg_system_close(self.0) };
    }
}

/// Page-aligned memory for the grabber to fill.
struct Memory {
    ptr: *mut u8,
    layout: Layout,
}

impl Memory {
    fn new(size: usize) -> Result<Self> {
        let layout = Layout::from_size_align(size, 4096).map_err(|error| error.to_string())?;
        // SAFETY: the size isn't zero: it is a frame's payload.
        let ptr = unsafe { alloc_zeroed(layout) };
        match ptr.is_null() {
            true => Err(format!("no memory for a buffer of {size} bytes")),
            false => Ok(Self { ptr, layout }),
        }
    }
}

impl Drop for Memory {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout; the grabber has revoked it.
        unsafe { dealloc(self.ptr, self.layout) };
    }
}

/// A camera, opened: the settings it changed, to put back, and the buffers it announced.
struct Grabber<'s> {
    raw: *mut SiqEgGrabber,
    changed: Vec<(SiqEgModule, String, String)>,
    // Dropped after the grabber is closed, in `drop`.
    buffers: Vec<Memory>,
    _system: PhantomData<&'s System>,
}

impl<'s> Grabber<'s> {
    fn open(system: &'s System, camera: i32) -> Result<Self> {
        let mut raw = ptr::null_mut();
        // SAFETY: the system is open and discovered, and `raw` is writable.
        check(
            unsafe { siq_eg_open(system.0, camera, &mut raw) },
            "opening the camera",
        )?;
        Ok(Self {
            raw,
            changed: Vec::new(),
            buffers: Vec::new(),
            _system: PhantomData,
        })
    }

    fn get(&self, module: SiqEgModule, name: &str) -> Result<String> {
        let feature = c(name)?;
        read(name, |buf, len, needed| {
            // SAFETY: the grabber is open; `read` passes a buffer writable for `len` bytes.
            unsafe { siq_eg_get_string(self.raw, module, feature.as_ptr(), buf, len, needed) }
        })
    }

    fn number(&self, module: SiqEgModule, name: &str) -> Result<f64> {
        let text = self.get(module, name)?;
        text.parse()
            .map_err(|_| format!("{name} is `{text}`, not a number"))
    }

    fn entries(&self, module: SiqEgModule, name: &str) -> Result<Vec<String>> {
        let feature = c(name)?;
        let list = read(name, |buf, len, needed| {
            let what = SIQ_EG_LIST_ENUM_ENTRIES;
            // SAFETY: as in `get`.
            unsafe { siq_eg_list(self.raw, module, what, feature.as_ptr(), buf, len, needed) }
        })?;
        Ok(list.lines().map(str::to_owned).collect())
    }

    /// Sets a feature, remembering its first value to put back.
    fn set(&mut self, module: SiqEgModule, name: &str, value: &str) -> Result<()> {
        let original = self.get(module, name)?;
        if !self
            .changed
            .iter()
            .any(|(m, n, _)| *m == module && n == name)
        {
            self.changed.push((module, name.to_owned(), original));
        }
        let (feature, value) = (c(name)?, c(value)?);
        // SAFETY: the grabber is open, and both strings are NUL-terminated.
        let status =
            unsafe { siq_eg_set_string(self.raw, module, feature.as_ptr(), value.as_ptr()) };
        check(status, &format!("setting {name}"))
    }

    fn execute(&self, module: SiqEgModule, name: &CStr) -> Result<()> {
        // SAFETY: the grabber is open, and `name` is NUL-terminated.
        check(
            unsafe { siq_eg_execute(self.raw, module, name.as_ptr()) },
            "a command",
        )
    }

    /// Allocates `count` buffers of `size` bytes, and announces them.
    fn announce(&mut self, count: usize, size: usize) -> Result<()> {
        self.buffers = (0..count)
            .map(|_| Memory::new(size))
            .collect::<Result<_>>()?;
        let ptrs: Vec<*mut c_void> = self.buffers.iter().map(|m| m.ptr.cast()).collect();
        // SAFETY: each buffer is `size` bytes, and lives until after the grabber is closed.
        let status = unsafe { siq_eg_announce_user_buffers(self.raw, ptrs.as_ptr(), size, count) };
        check(status, "announcing the buffers")
    }

    /// The next frame, or `None` if none came within `timeout`.
    fn pop(&self, timeout: Duration) -> Result<Option<SiqBufferInfo>> {
        let mut info = SiqBufferInfo {
            index: 0,
            ptr: ptr::null_mut(),
            size: 0,
            timestamp: 0,
            frame_id: 0,
            width: 0,
            height: 0,
            pixfmt: 0,
            flags: 0,
            pixfmt_namespace: 0,
        };
        let ms = timeout.as_millis() as u64;
        // SAFETY: the grabber is open, and `info` is writable.
        match unsafe { siq_eg_pop(self.raw, ms, &mut info) } {
            SIQ_EG_TIMEOUT => Ok(None),
            status => check(status, "waiting for a frame").map(|()| Some(info)),
        }
    }

    fn requeue(&self, index: u64) -> Result<()> {
        // SAFETY: the grabber is open, and `index` was popped.
        check(
            unsafe { siq_eg_requeue(self.raw, index) },
            "requeuing a buffer",
        )
    }

    fn start(&self) -> Result<()> {
        // SAFETY: the grabber is open, with buffers announced.
        check(unsafe { siq_eg_start(self.raw, 0) }, "starting")
    }

    fn stop(&self) -> Result<()> {
        // SAFETY: the grabber is open.
        check(unsafe { siq_eg_stop(self.raw) }, "stopping")
    }

    fn counters(&self) -> Result<SiqEgCounters> {
        let mut counters = SiqEgCounters {
            announced: 0,
            queued: 0,
            awaiting_delivery: 0,
            delivered: 0,
            underrun: 0,
            started: 0,
        };
        // SAFETY: the grabber is open, and `counters` is writable.
        check(
            unsafe { siq_eg_counters(self.raw, &mut counters) },
            "the counters",
        )?;
        Ok(counters)
    }

    /// The frame in buffer `info.index`, as 8- or 16-bit pixels.
    fn pixels(&self, info: &SiqBufferInfo) -> Result<Pixels<'_>> {
        let memory = &self.buffers[info.index as usize];
        let count = info.width as usize * info.height as usize;
        let bytes = match (info.pixfmt >> 16) & 0xff {
            8 => 1,
            16 => 2,
            bits => return Err(format!("{bits} bits a pixel: neither 8 nor 16")),
        };
        if (info.size as usize) < count * bytes || memory.layout.size() < count * bytes {
            return Err(format!("{} bytes filled, for {count} pixels", info.size));
        }
        // SAFETY: the grabber filled `count` pixels of `bytes` there, within the buffer, and it is
        // stopped: nothing writes them while borrowed. The memory is page-aligned, for `u16`.
        Ok(unsafe {
            match bytes {
                1 => Pixels::Eight(std::slice::from_raw_parts(memory.ptr, count)),
                _ => Pixels::Sixteen(std::slice::from_raw_parts(memory.ptr.cast(), count)),
            }
        })
    }
}

impl Drop for Grabber<'_> {
    fn drop(&mut self) {
        let _ = self.stop();
        // Announcing nothing revokes the buffers, so that every setting can be put back.
        // SAFETY: the grabber is open, and stopped.
        unsafe { siq_eg_announce_user_buffers(self.raw, ptr::null(), 0, 0) };
        for (module, name, value) in self.changed.drain(..).rev() {
            let (Ok(feature), Ok(text)) = (c(&name), c(&value)) else {
                continue;
            };
            // SAFETY: as in `set`.
            let status =
                unsafe { siq_eg_set_string(self.raw, module, feature.as_ptr(), text.as_ptr()) };
            match status {
                SIQ_EG_OK => println!("  put back {name} = {value}"),
                _ => eprintln!("  couldn't put back {name} = {value}: {}", last_error()),
            }
        }
        // SAFETY: the grabber is open; its buffers are revoked and freed after this.
        unsafe { siq_eg_close(self.raw) };
    }
}

enum Pixels<'a> {
    Eight(&'a [u8]),
    Sixteen(&'a [u16]),
}

/// The deepest format of `current`'s kind among `entries`: `BayerGB12` for `BayerGB8`. Packed
/// variants (`BayerGB12p`) are left out.
fn deepest(current: &str, entries: &[String]) -> Option<String> {
    let kind = current.trim_end_matches(|c: char| c.is_ascii_digit());
    let bits = |entry: &String| entry.strip_prefix(kind)?.parse::<u32>().ok();
    let deepest = entries
        .iter()
        .filter_map(|entry| Some((bits(entry)?, entry)))
        .max()?;
    Some(deepest.1.clone())
}

/// The bits a pixel's value has, from its format's name: 12 for `BayerGB12`.
fn significant(format: &str) -> u32 {
    let digits = format.trim_start_matches(|c: char| !c.is_ascii_digit());
    digits.parse().unwrap_or(16)
}

struct Options {
    seconds: f64,
    exposure_us: Option<f64>,
    depth: usize,
    overlap: bool,
    camera: i32,
    save: Option<PathBuf>,
}

fn options() -> Result<Options> {
    let mut options = Options {
        seconds: 10.0,
        exposure_us: None,
        depth: 2,
        overlap: false,
        camera: 0,
        save: None,
    };
    let mut args = std::env::args().skip(1);
    fn value<T: std::str::FromStr>(
        args: &mut impl Iterator<Item = String>,
        name: &str,
    ) -> Result<T> {
        let text = args.next().ok_or(format!("{name} needs a value"))?;
        text.parse()
            .map_err(|_| format!("{name} can't be `{text}`"))
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => options.seconds = value(&mut args, "--seconds")?,
            "--exposure-us" => options.exposure_us = Some(value(&mut args, "--exposure-us")?),
            "--depth" => options.depth = value(&mut args, "--depth")?,
            "--overlap" => options.overlap = true,
            "--camera" => options.camera = value(&mut args, "--camera")?,
            "--save" => options.save = Some(value(&mut args, "--save")?),
            folder if !folder.starts_with('-') => options.save = Some(PathBuf::from(folder)),
            other => {
                return Err(format!(
                    "unknown option {other}: see the example's documentation"
                ));
            }
        }
    }
    if options.depth == 0 || options.seconds <= 0.0 {
        return Err("--depth and --seconds must be above 0".into());
    }
    Ok(options)
}

/// What one frame told us.
struct Frame {
    timestamp: u64,
    frame_id: u64,
    latency: Duration,
}

/// The mean, least and greatest of `values`.
fn spread(values: impl Iterator<Item = f64>) -> (f64, f64, f64) {
    let (mut sum, mut count, mut least, mut most) = (0.0, 0, f64::INFINITY, f64::NEG_INFINITY);
    for value in values {
        (sum, count) = (sum + value, count + 1);
        (least, most) = (least.min(value), most.max(value));
    }
    (sum / f64::from(count.max(1)), least, most)
}

fn save(folder: &Path, pixels: Pixels, info: &SiqBufferInfo, description: &str) -> Result<PathBuf> {
    use tiff::encoder::{TiffEncoder, colortype};
    use tiff::tags::Tag;
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let path = folder.join(format!("eg_fps_frame_{}.tif", info.frame_id));
    let file = std::fs::File::create(&path).map_err(|error| error.to_string())?;
    let mut encoder = TiffEncoder::new(std::io::BufWriter::new(file)).map_err(|e| e.to_string())?;
    let (width, height) = (info.width, info.height);
    let written = match pixels {
        Pixels::Eight(data) => encoder
            .new_image::<colortype::Gray8>(width, height)
            .and_then(|mut image| {
                image
                    .encoder()
                    .write_tag(Tag::ImageDescription, description)?;
                image.write_data(data)
            }),
        Pixels::Sixteen(data) => encoder
            .new_image::<colortype::Gray16>(width, height)
            .and_then(|mut image| {
                image
                    .encoder()
                    .write_tag(Tag::ImageDescription, description)?;
                image.write_data(data)
            }),
    };
    written.map_err(|error| error.to_string())?;
    Ok(path)
}

fn run() -> Result<()> {
    let options = options()?;
    let (system, cameras) = System::open()?;
    if options.camera >= cameras {
        return Err(format!(
            "camera {} asked for, {cameras} found",
            options.camera
        ));
    }
    let mut grabber = Grabber::open(&system, options.camera)?;
    let model = grabber.get(SIQ_EG_REMOTE, "DeviceModelName")?;

    // The camera: its deepest format, triggered over CoaXPress, exposure timed by itself.
    let current = grabber.get(SIQ_EG_REMOTE, "PixelFormat")?;
    let formats = grabber.entries(SIQ_EG_REMOTE, "PixelFormat")?;
    let format =
        deepest(&current, &formats).ok_or(format!("no format like {current}: {formats:?}"))?;
    grabber.set(SIQ_EG_REMOTE, "PixelFormat", &format)?;
    grabber.set(SIQ_EG_REMOTE, "ExposureMode", "TimedTriggerControl")?;
    grabber.set(SIQ_EG_REMOTE, "TriggerSource", "Trigger")?;
    if let Some(exposure) = options.exposure_us {
        grabber.set(SIQ_EG_REMOTE, "ExposureTime", &exposure.to_string())?;
    }
    let exposure = grabber.number(SIQ_EG_REMOTE, "ExposureTime")?;

    // The grabber: a trigger at each StartCycle, the next cycle once the readout ends.
    grabber.set(SIQ_EG_DEVICE, "CameraControlMethod", "RC")?;
    grabber.set(SIQ_EG_DEVICE, "CycleTriggerSource", "StartCycle")?;
    grabber.set(SIQ_EG_DEVICE, "ExposureTime", &exposure.to_string())?;
    let overlap = if options.overlap { "True" } else { "False" };
    grabber.set(SIQ_EG_DEVICE, "ExposureReadoutOverlap", overlap)?;
    let pending = (options.depth - 1).to_string();
    grabber.set(SIQ_EG_DEVICE, "CycleMaxPendingTriggerCount", &pending)?;

    // Raw: the camera's format, its values in the low bits of 16, one component a pixel.
    grabber.set(SIQ_EG_STREAM, "UnpackingMode", "Lsb")?;
    let delivered = grabber.get(SIQ_EG_STREAM, "PixelFormat")?;
    let components = grabber.number(SIQ_EG_STREAM, "PixelComponentCount")?;
    if delivered != format || components != 1.0 {
        return Err(format!(
            "the grabber would deliver {delivered}, {components} components"
        ));
    }
    let payload = grabber.number(SIQ_EG_STREAM, "PayloadSize")? as usize;
    let (width, height) = (
        grabber.get(SIQ_EG_REMOTE, "Width")?,
        grabber.get(SIQ_EG_REMOTE, "Height")?,
    );
    let link = grabber.get(SIQ_EG_DEVICE, "CxpLinkConfiguration")?;
    println!("camera   {model}, {width} × {height}, on {link}");
    println!(
        "format   {format} (was {current}), raw, unpacked to the low bits; {payload} bytes a frame"
    );
    println!("exposure {exposure} µs");
    let overlapping = if options.overlap {
        ", overlapping readout"
    } else {
        ""
    };
    println!(
        "trigger  software: StartCycle on the Coaxlink, {} in flight{overlapping}",
        options.depth
    );

    grabber.announce(options.depth + 2, payload)?;
    let lost_before = grabber.number(SIQ_EG_DEVICE, "CycleLostTriggerCount")?;
    let rejected_before = grabber.number(SIQ_EG_STREAM, "DiagnosticRejectedFrameCount")?;
    grabber.start()?;

    let timeout = Duration::from_secs_f64(exposure / 1e6) + Duration::from_secs(2);
    let started = Instant::now();
    let until = started + Duration::from_secs_f64(options.seconds);
    let (mut triggered, mut timeouts, mut incomplete) = (0u64, 0u64, 0u64);
    let mut in_flight: VecDeque<Instant> = VecDeque::new();
    let mut frames: Vec<Frame> = Vec::new();
    let mut last: Option<SiqBufferInfo> = None;
    loop {
        while Instant::now() < until && in_flight.len() < options.depth {
            grabber.execute(SIQ_EG_DEVICE, c"StartCycle")?;
            triggered += 1;
            in_flight.push_back(Instant::now());
        }
        let Some(sent) = in_flight.pop_front() else {
            break;
        };
        let Some(info) = grabber.pop(timeout)? else {
            timeouts += 1;
            continue;
        };
        if info.pixfmt_namespace != 4 || (info.pixfmt >> 24) & 0x7f != 1 {
            return Err(format!("a frame in format {:#x}, not raw", info.pixfmt));
        }
        incomplete += u64::from(info.flags & SIQ_EG_BUFFER_INCOMPLETE != 0);
        let (timestamp, frame_id, latency) = (info.timestamp, info.frame_id, sent.elapsed());
        frames.push(Frame {
            timestamp,
            frame_id,
            latency,
        });
        if let Some(previous) = last.replace(info) {
            grabber.requeue(previous.index)?;
        }
    }
    let wall = started.elapsed();
    grabber.stop()?;

    let counters = grabber.counters()?;
    let lost = grabber.number(SIQ_EG_DEVICE, "CycleLostTriggerCount")? - lost_before;
    let rejected = grabber.number(SIQ_EG_STREAM, "DiagnosticRejectedFrameCount")? - rejected_before;
    let gaps = frames
        .windows(2)
        .filter(|pair| pair[1].frame_id != pair[0].frame_id + 1)
        .count();
    println!(
        "\n{:.1} s: {triggered} triggers, {} frames, {timeouts} timed out",
        wall.as_secs_f64(),
        frames.len()
    );
    if let [first, .., end] = frames.as_slice() {
        let span = (end.timestamp - first.timestamp) as f64 / 1e9;
        let intervals = frames
            .windows(2)
            .map(|p| (p[1].timestamp - p[0].timestamp) as f64 / 1e6);
        let (mean, least, most) = spread(intervals);
        let rate = (frames.len() - 1) as f64 / span;
        println!(
            "rate     {rate:.3} frames/s by the grabber's clock: every {mean:.2} ms ({least:.2}–{most:.2})"
        );
        println!(
            "         {:.3} frames/s by the wall clock",
            frames.len() as f64 / wall.as_secs_f64()
        );
        let (mean, least, most) = spread(frames.iter().map(|f| f.latency.as_secs_f64() * 1e3));
        println!(
            "latency  {mean:.1} ms from StartCycle to the frame in memory ({least:.1}–{most:.1})"
        );
    }
    println!(
        "lost     {lost} triggers in the grabber, {rejected} frames rejected, {} underruns, {gaps} frame-id gaps, {incomplete} incomplete",
        counters.underrun
    );

    if let (Some(folder), Some(info)) = (&options.save, &last) {
        let description = format!(
            "{model}: {format}, the raw mosaic, not debayered; {} significant bits in {}; exposure {exposure} us; frame {}, at {} ns",
            significant(&format),
            (info.pixfmt >> 16) & 0xff,
            info.frame_id,
            info.timestamp
        );
        let path = save(folder, grabber.pixels(info)?, info, &description)?;
        println!("saved    {}", path.display());
    }
    println!("settings");
    drop(grabber);
    drop(system);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eg_fps: {error}");
            ExitCode::FAILURE
        }
    }
}
