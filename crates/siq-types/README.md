# siq-types

SpectrumIQ's shared vocabulary:

- units;
- coordinate frames and transforms;
- image views;
- frame metadata;
- ids and clocks.

Every other crate speaks these types. Their conventions are fixed by
[ADR-0018](../../docs/adr/0018-coordinate-frames-units-and-handedness.md).

## Place in the architecture

- **Layer:** L0 Foundation.
- **Depends on:** `serde`, `schemars` (with `uuid1`), `uuid` (v7), `chrono`, `thiserror`.
- **Used by:** every crate.
- **Phase:** P0.2. P1.7 adds `Clock::sleep_until`.

## API

| Module | Types | Notes |
|---|---|---|
| `units` | `Micrometers`, `Millimeters`, `Nanometers`, `Millis`, `Hertz` | `f64` newtypes. They add and subtract with themselves, scale by `f64`, divide to a ratio, convert, and print with their symbol (`{:.2}` → `1.50 µm`) |
| `geometry` | `Vec2Um`, `PointPx`, `RectUm`, `RectPx`, `StagePose` | Stage-frame µm and image pixels. Pixel centres are at half-integers |
| `affine` | `Affine2`, `AffineParams`, `StageToPixel`, `PixelToStage` | `compose`, `invert`, `from_params` and `decompose`. `StageToPixel::for_frame` is the calibration convention |
| `image` | `PixelFormat`, `Roi`, `Binning`, `ImageView`, `ImageViewMut`, `OwnedImage` | Views borrow any memory, with a stride, and never allocate. `OwnedImage` rows are 64-byte aligned |
| `frame` | `FrameMeta`, `FrameSeq`, `DeviceMicros`, `PoseQuality`, `FrameFlags` | `FrameMeta` is `Copy`, with no heap data |
| `ids` | `DatasetId`, `CalibrationId`, `ChannelId`, `ObjectiveId` | UUID v7 for run-time things, configured `u16` numbers for channels and objectives |
| `time` | `Timestamp`, `Clock`, `SystemClock`, `ManualClock` | One shared `Arc<dyn Clock>` per service; `ManualClock` for deterministic tests |

Every type that crosses the HTTP API or a file derives `Serialize`, `Deserialize` and
`JsonSchema`. The JSON forms are pinned by snapshots in `tests/snapshots/`.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `affine_demo` | A 0.345 µm/px calibration, slightly rotated, with `y` flipped. Stage points are mapped to pixels for a full frame and for a binned ROI, and back. The example checks its own results | `cargo run -p siq-types --example affine_demo` |

## Features

None.

## Testing

`cargo test -p siq-types` covers:

- **Unit tests** in `src/*/tests.rs`:
  - unit arithmetic and conversions;
  - geometry;
  - affine property tests (inverse, composition, decomposition, any invertible transform rebuilt
    from its parameters);
  - the stage↔pixel convention with a ROI and binning;
  - image view layout, strides, sub-views, mutable views, alignment, the zero-width edge case;
  - clocks.
- **Integration tests** in `tests/serde.rs`:
  - JSON round trips of generated frame metadata and transforms;
  - insta snapshots of the frame-metadata JSON and the JSON Schemas;
  - id parsing.

**Snapshots.** After a deliberate format change, regenerate them with
`INSTA_UPDATE=always cargo test -p siq-types --test serde` and review the diff.

## Limitations

- **`Clock` has no `sleep_until` yet.** P1.7 adds it, with its first user.
- **`OwnedImage` aligns rows** only when the pixel size divides 64 bytes. It is for tests and
  tools, not the frame path.
- **`Binning::image_size` drops partial bins** at the edges, as most cameras do.
