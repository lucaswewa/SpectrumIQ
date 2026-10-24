# Software of unknown provenance (SOUP) register

This register lists every third-party component SpectrumIQ ships or builds with (REQ-NFR-006).

- **Rows are added** by the phase that introduces the component.
- **`cargo xtask soup`** (P15.3) generates the full list from `cargo metadata` and `npm ls`, and
  diffs it against this file.
- **Licences** must follow the policy in [plan/conventions.md §7](plan/conventions.md#7-dependency-policy).

| Component | Version | Licence | Purpose | Used by | Introduced in | Notes |
|---|---|---|---|---|---|---|
| teta-wot | v0.1.0 (`135f9ff`) | MIT | Web of Things server framework | `siq-server` | P2.3 | Git dependency (ADR-0003) |
| Euresys eGrabber | 25.02.0.40 | Proprietary (Euresys EULA) | Coaxlink frame grabber SDK | `siq-euresys-sys` | P2.5 | Build-time and runtime prerequisite. Never vendored (ADR-0004) |
| microscope web app | `0640f99` | MIT | Starting point of the frontend (theme, ui, host, api/wot, app shell) | `web/` | P3.1 | Copied and adapted (ADR-0008) |
| Rust toolchain | 1.99.1 | MIT OR Apache-2.0 | Compiler, rustfmt, clippy | everything | P0.1 | Pinned in `rust-toolchain.toml` (ADR-0017) |
| anyhow | 1.0.104 | MIT OR Apache-2.0 | Error handling in tools | `xtask` | P0.1 | Binaries and xtask only (conventions §5.1) |
| clap | 4.6.7 | MIT OR Apache-2.0 | Command-line parsing | `xtask` | P0.1 | Same version as teta-wot |
| serde | 1.0.229 | MIT OR Apache-2.0 | Serialization | `xtask` | P0.1 | Reads `cargo metadata` and `package.json` |
| serde_json | 1.0.151 | MIT OR Apache-2.0 | JSON | `xtask`; `siq-types` (dev) | P0.1 | 1.0.152–1.0.154 were under two weeks old; pinned back. `float_roundtrip` on since P0.2 (ADR-0006) |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Scratch folders in tests | `xtask` (dev) | P0.1 | Test-only |
| chrono | 0.4.45 | MIT OR Apache-2.0 | UTC times for the `Clock` | `siq-types` | P0.2 | Same version as teta-wot |
| schemars | 1.2.2 | MIT | JSON Schemas of the shared types | `siq-types` | P0.2 | 1.2.3 was under two weeks old; pinned back. `uuid1` feature for the ids |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | Error enums | `siq-types` | P0.2 | Same version as teta-wot |
| uuid | 1.27.0 | Apache-2.0 OR MIT | UUID v7 dataset and calibration ids | `siq-types` | P0.2 | 1.28 and 1.29 were under two weeks old; pinned back |
| proptest | 1.11.0 | MIT OR Apache-2.0 | Property tests | `siq-types` (dev) | P0.2 | 1.12.0 was under two weeks old; pinned back. Its `zerocopy` pinned to 0.8.59 |
| insta | 1.49.0 | Apache-2.0 | Snapshot tests | `siq-types` (dev) | P0.2 | Published 2026-10-03 |
| tracing | 0.1.44 | MIT | Structured logging | `siq-telemetry`, then every crate | P0.3a | Same version as teta-wot |
| tracing-subscriber | 0.3.23 | MIT | Console and file output, env filter | `siq-telemetry` | P0.3a | Features `std`, `registry`, `fmt`, `ansi`, `env-filter`, as teta-wot |
| tracing-appender | 0.2.5 | MIT | Daily rolling log files, non-blocking writer | `siq-telemetry` | P0.3a | Brings `time` 0.3.55; its `deranged` pinned to 0.5.8 (0.5.9 was under two weeks old) |
| toml | 1.1.6 | MIT OR Apache-2.0 | The instrument configuration's format, with spans | `siq-config` | P0.3b | teta-wot's version. Its `toml_parser` 1.1.3, `toml_datetime` 1.1.1, `toml_writer` 1.1.2 and `serde_spanned` 1.1.1 are pinned to teta-wot's, the newer ones being 14–15 days old. `winnow` 1.0.4 is MIT |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | Windows API: `VirtualAlloc`, `VirtualLock`, the working set | `siq-buffer` (direct, since P1.1) | P1.1 | Already in the lockfile through tempfile and clap, and teta-wot's version. Features `Win32_Foundation`, `Win32_System_Memory`, `Win32_System_Threading`. With `windows-link` 0.2.1 |
| tokio | 1.53.1 | MIT | `watch` channels for device status; later the async runtime | `siq-devices` (`sync`), then the engines and the service | P1.3 | teta-wot's version. Cargo had resolved 1.54.1, two days old. The workspace entry names no features; each crate names its own |
| tokio-macros | 2.7.2 | MIT | `#[tokio::test]`, `#[tokio::main]` | `siq-devices` (dev) | P1.3 | teta-wot's version. Test and example only, for now |
| png | 0.18.1 | MIT OR Apache-2.0 | PNG decoding (image worlds) and encoding (examples) | `siq-synth` | P1.4 | Published 2026-02-14. With `fdeflate` 0.3.7 (MIT OR Apache-2.0), `crc32fast` 1.5.2 (MIT OR Apache-2.0; 1.5.3 was 3 days old, pinned back) and `bitflags` |
| tiff | 0.11.3 | MIT | TIFF decoding (image worlds); later OME-TIFF | `siq-synth` | P1.4 | Published 2026-02-10. Features `deflate` and `lzw` only: no JPEG, fax or WebP. With `half` 2.7.1 and `crunchy` 0.2.4, `quick-error` 2.0.1, `weezl` 0.1.12 (all MIT or MIT OR Apache-2.0) |
| flate2 | 1.1.9 | MIT OR Apache-2.0 | Deflate, for PNG and TIFF | `png`, `tiff` | P1.4 | Pinned from 1.1.10 so that `png` and `tiff` share one `miniz_oxide` 0.8.9 (MIT OR Zlib OR Apache-2.0), with `adler2` 2.0.1 and `simd-adler32` 0.3.10 (MIT) |
| rayon | 1.12.0 | MIT OR Apache-2.0 | Data parallelism: row bands of the CPU kernels | `siq-imgproc`, then the processing and job crates | P1.5 | Published 2026-04-14. With `rayon-core` 1.13.0, `crossbeam-deque` 0.8.8, `crossbeam-epoch` 0.9.21 and `either` 1.19.0 (18 days old), all MIT OR Apache-2.0. `crossbeam-utils` 0.8.23 was already in the lockfile, through `tracing-appender` |
| tokio-serial | 5.5.0 | MIT | Serial ports for tokio | `siq-serial` | P1.8 | Published 2026-06-15. No default features. With `mio-serial` 5.0.7 (MIT) |
| serialport | 4.10.1 | MPL-2.0 | The serial port underneath `tokio-serial` | `mio-serial` | P1.8 | Used unmodified (ADR-0028). Brings a second `windows-sys`, 0.52.0 (MIT OR Apache-2.0), and `scopeguard` 1.2.0 |
| mio | 1.2.4 | MIT | tokio's I/O driver, for serial ports and the `net` feature | `tokio`, `mio-serial` | P1.8 | Published 2026-10-03 (21 days). With `socket2` 0.6.5, `bytes` 1.12.1 and `futures-sink` 0.3.34 |
| regex | 1.13.1 | MIT OR Apache-2.0 | The regexes that dialect templates compile to | `siq-ascii` | P1.10a | Published 2026-07-15. Default features. `regex-automata` 0.4.18, `regex-syntax` 0.8.11, `aho-corasick` 1.1.5 and `memchr` 2.8.3 were already in the lockfile |
| jpeg-encoder | 0.7.1 | (MIT OR Apache-2.0) AND IJG | JPEG encoding of the live previews | `siq-preview` | P2.2 | Published 2026-07-27. Feature `simd` (AVX2, no dependencies). IJG allowed with its attribution (ADR-0028), which is in the README; the About page follows with the frontend |
