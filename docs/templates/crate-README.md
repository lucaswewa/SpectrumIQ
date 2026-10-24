# <crate-name>

<!-- The README of every crate and app (conventions §4.3). Replace each <…> and keep the sections. -->

<What the crate is for, in 2–3 lines.>

## Place in the architecture

- **Layer:** <L0 Foundation | L1 Core | L2 Devices | L3 Processing | L4 Engines | L5 Service | App>
- **Depends on:** <workspace crates, and notable external ones>
- **Used by:** <workspace crates>
- **Phase:** <P<m>.<n> created it; later phases that changed it>

## API

<The main types and functions, and how they fit together. Link to the rustdoc for the details.>

## Examples

| Example | What it shows | Run |
|---|---|---|
| `<name>` | <…> | `cargo run -p <crate> --example <name>` |

Every example also runs in smoke mode (`SIQ_SMOKE=1`) in `cargo xtask ci`.

## Features

<Cargo features and what they enable, or "None".>

## Testing

<What the tests cover, and how to run any hardware or performance tests, such as
`cargo test -p <crate> -- --ignored` with `SIQ_…` set.>

## Limitations

<Known limitations and planned follow-ups.>
