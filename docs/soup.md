# Software of unknown provenance (SOUP) register

This register lists every third-party component SpectrumIQ ships or builds with (REQ-NFR-006).

- **Rows are added** by the phase that introduces the component.
- **`cargo xtask soup`** (P15.3) generates the full list from `cargo metadata` and `npm ls`, and
  diffs it against this file.
- **Licences** must follow the policy in [plan/conventions.md §7](plan/conventions.md#7-dependency-policy).

| Component | Version | Licence | Purpose | Used by | Introduced in | Notes |
|---|---|---|---|---|---|---|
| teta-wot | v0.1.0 (`135f9ff`) | MIT | Web of Things server framework | `siq-server` | P2.3 | Git dependency (ADR-0003), with no features but `testing` in the tests. Its crates' manifests have no `license` field; the repository's LICENSE is MIT. cargo-deny will need a `[[licenses.clarify]]` for them |
| Euresys eGrabber | 25.02.0.40 | Proprietary (Euresys EULA) | Coaxlink frame grabber SDK | `siq-euresys-sys` | P2.5 | Build-time and runtime prerequisite. Never vendored (ADR-0004); `cargo xtask ci` fails on a tracked file named as one of its files. With the `sdk` feature only |
| cc | 1.5.1 | MIT OR Apache-2.0 | Compiles the Euresys shim with MSVC | `siq-euresys-sys` (build, with `sdk`) | P2.5 | Already in the lockfile through `iana-time-zone-haiku`. With `find-msvc-tools` 0.1.14 and `shlex` 2.0.1 (both MIT OR Apache-2.0) |
| microscope web app | `0640f99` | MIT | Starting point of the frontend (theme, ui, host, api/wot, app shell) | `web/` | P3.1 | Copied and adapted (ADR-0008): 97 files in the import commit `6f83351`, listed in P3.1's notes. Credited in `NOTICE` |
| rust-embed | 8.13.0 | MIT | The web app's build, embedded in release builds of the server | `siq-server` | P3.1 | ADR-0036. With `rust-embed-impl` and `rust-embed-utils` 8.13.0 (MIT), `walkdir` 2.5.0, `same-file` 1.0.6 and `winapi-util` 0.1.11 (Unlicense OR MIT), `mime_guess` 2.0.5 (MIT), `unicase` 2.10.0, `sha2` 0.11.0, `digest` 0.11.3, `block-buffer` 0.12.1, `crypto-common` 0.2.2, `hybrid-array` 0.4.15, `const-oid` 0.10.2 and `cpufeatures` 0.3.1 (MIT OR Apache-2.0). `sha2` is pinned to 0.11.0: 0.11.1 is too new for the two-week rule |
| vue | 3.5.43 | MIT | The web app's framework | `web/` (bundled) | P3.1 | With `@vue/runtime-core`, `runtime-dom`, `reactivity` and `shared` |
| vue-router | 5.3.1 | MIT | Hash routing | `web/` (bundled) | P3.1 | With `@vue/devtools-api`, `devtools-kit` and `devtools-shared` 8.2.1, `birpc`, `hookable`, `perfect-debounce` (MIT) |
| pinia | 4.0.3 | MIT | Stores | `web/` (bundled) | P3.1 | |
| reka-ui | 2.10.5 | MIT | The primitives under `src/ui/`'s wrappers (ADR-0008, amended) | `web/` (bundled) | P3.1 | With `@floating-ui/*`, `@tanstack/vue-virtual`, `@vueuse/core`, `aria-hidden`, `defu`, `ohash`, `nostics`, `vue-demi` (MIT), and `@internationalized/date` and `number` (Apache-2.0). `tslib` (0BSD) is in its tree, not in the bundle |
| @material-symbols/svg-400 | 0.47.5 | Apache-2.0 | Icons, and the favicon | `web/` (bundled) | P3.1 | Credited in `NOTICE` |
| Web build and test tools | see `web/package-lock.json` | MIT, Apache-2.0, ISC, BSD, BlueOak-1.0.0, MPL-2.0 | Vite 8.3.1, Vitest 5.0.1, TypeScript 6.0.3, vue-tsc 3.3.11, ESLint 10.11.0, Prettier 3.9.9, happy-dom 20.14.5, Vue Test Utils 2.5.1, Playwright 1.63.0 | `web/` (development only) | P3.1 | Not bundled. At `microscope`'s locked versions, installed with `--before=2026-10-10`. `lightningcss` 1.33.0, Vite's CSS processor, is MPL-2.0 and used unmodified (ADR-0028); `glob`, `lru-cache`, `minimatch`, `minipass` and `path-scurry` are BlueOak-1.0.0 (ADR-0028, amended) |
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
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | Windows API: `VirtualAlloc`, `VirtualLock`, the working set | `siq-buffer` (direct, since P1.1) | P1.1 | Already in the lockfile through tempfile and clap, and teta-wot's version. Features `Win32_Foundation`, `Win32_System_Memory`, `Win32_System_Threading`, and since P2.4 `Win32_System_ProcessStatus` (`K32GetProcessMemoryInfo`). With `windows-link` 0.2.1 |
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
| axum | 0.8.9 | MIT | The HTTP server under teta-wot, and SpectrumIQ's routes (through teta-wot's re-export) | teta-wot, `siq-server` | P2.3 | teta-wot's version and features. With `hyper` 1.11.1 (pinned back from 1.12.0 to teta-wot's), `hyper-util` 0.1.21, `http` 1.5.0, `matchit` 0.8.4 (MIT AND BSD-3-Clause), `tokio-tungstenite` and `tungstenite` 0.29.0, `sha1_smol` 1.0.1 (BSD-3-Clause), `ryu` 1.0.23 (Apache-2.0 OR BSL-1.0): every new package at teta-wot's locked version |
| tower | 0.5.3 | MIT | `ServiceExt::oneshot`, to read the OpenAPI document without serving | `siq-server` | P2.3 | teta-wot's version. Feature `util` |
| smallvec | 1.16.2 | MIT OR Apache-2.0 | Small vectors, inside `hyper` | `hyper` | P2.3 | Pinned back from 1.16.4, a day old, to teta-wot's |
| http-body-util | 0.1.5 | MIT | Reading the MJPEG stream's parts in the server's tests | `siq-server` (dev) | P2.3 | teta-wot's version |
