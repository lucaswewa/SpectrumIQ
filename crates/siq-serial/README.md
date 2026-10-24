# siq-serial

SpectrumIQ's line-oriented serial transports, for:

- the GRBL stage;
- the focus drive;
- the ASCII peripherals.

What it provides:

- **`LineTransport`:** write a line, write a realtime byte, read a line within a timeout.
- **`LineStream`:** that trait over any byte stream, with configurable framing.
- **`SerialTransport`:** a serial port, through `tokio-serial`.
- **`DuplexTransport`:** an in-memory link to an emulator, which serves the other end.

The design is in
[ADR-0027](../../docs/adr/0027-protocol-level-emulation-and-the-serial-transport.md).

## Place in the architecture

- **Layer:** L2 Devices.
- **Depends on:**
  - `tokio` (`io-util`, `time`);
  - `tokio-serial`, which brings `serialport` (MPL-2.0, used unmodified: ADR-0028);
  - `tracing`, `thiserror`.
- **Used by:**
  - P1.9's GRBL driver;
  - P1.10's ASCII driver;
  - the emulators' tests in `siq-sim`.
- **Phase:** P1.8.

## API

| Item | What |
|---|---|
| `LineTransport` | `write_line(&str)`, `write_realtime(u8)`, `read_line(timeout)` → the line without its terminator. Its futures are `Send` |
| `SerialError` | `Timeout` (what arrived of a line is kept for the next read), `Closed`, `Io` |
| `Framing { write, read }` | The write terminator (default `\n`) and the read terminator (default `\n`; a `\r` before it is dropped) |
| `LineStream<S>` | The trait over any `AsyncRead + AsyncWrite` stream, traced at debug level by name |
| `duplex(framing, name)` → `(DuplexTransport, DuplexStream)` | The host's end of an in-memory link, and the device's end for an emulator |
| `open(port, &PortSettings, framing)` → `SerialTransport` | A serial port. `PortSettings::default()` is 115 200 baud 8N1, GRBL's |

```rust
let (mut stage, device) = duplex(Framing::default(), "grbl");
tokio::spawn(GrblEmulator::new(settings, errors, seed, state, clock).run(device));
stage.write_line("G1 X-10 F600").await?;
let reply = stage.read_line(Duration::from_secs(1)).await?;   // "ok"
```

## Examples

None. `siq-sim`'s `grbl_emulator_repl` uses a `DuplexTransport`.

## Features

None.

## Testing

`cargo test -p siq-serial` runs `lines`:

- framing;
- a line split across writes, with a timeout between;
- realtime bytes;
- a closed link;
- `\r` framing;
- a port that isn't there.

## Limitations

- **`LineTransport` isn't dyn-compatible:** drivers are generic over it.
- **A real port is tested only by hand,** from P1.9's `grbl_drive --port COMn`.
