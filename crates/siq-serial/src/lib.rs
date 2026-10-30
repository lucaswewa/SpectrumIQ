//! Line-oriented transports for the serial devices: the GRBL stage, and the focus drive and
//! peripherals of the ASCII dialects (ADR-0027).
//!
//! - [`LineTransport`]: write a line, write a realtime byte, read a line within a timeout.
//! - [`LineStream`] implements it over any byte stream, with a [`Framing`].
//! - [`open`] opens a serial port; [`duplex`] makes an in-memory link to an emulator, which gets
//!   the other end.
//! - [`ports`] lists the serial ports this computer has (P10.3).
//!
//! Every line and realtime byte sent, and every line received, is traced at debug level, with the
//! link's name.

use std::future::Future;
use std::io;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream};
use tokio_serial::{
    DataBits, FlowControl, Parity, SerialPortBuilderExt, SerialPortType, SerialStream, StopBits,
};
use tracing::debug;

/// A link that carries lines of text, and single realtime bytes.
pub trait LineTransport: Send {
    /// Sends `line`, then the line terminator.
    fn write_line(&mut self, line: &str) -> impl Future<Output = Result<(), SerialError>> + Send;

    /// Sends one byte alone, outside the lines: GRBL's `?`, `!`, `~` and 0x18.
    fn write_realtime(&mut self, byte: u8) -> impl Future<Output = Result<(), SerialError>> + Send;

    /// The next line, without its terminator, if one arrives within `timeout`.
    fn read_line(
        &mut self,
        timeout: Duration,
    ) -> impl Future<Output = Result<String, SerialError>> + Send;
}

/// Why a transport failed.
#[derive(Debug, thiserror::Error)]
pub enum SerialError {
    /// No whole line arrived in time. What arrived of one is kept for the next read.
    #[error("no reply in time")]
    Timeout,
    /// The other end closed the link.
    #[error("the link is closed")]
    Closed,
    /// The port failed.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// How lines are terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Framing {
    /// What follows each line sent.
    pub write: String,
    /// The byte that ends a line received. A `\r` before it is dropped too.
    pub read: u8,
}

impl Default for Framing {
    /// Lines sent end in `\n`, and lines received end in `\n`, as GRBL's do.
    fn default() -> Self {
        Self {
            write: "\n".into(),
            read: b'\n',
        }
    }
}

/// A [`LineTransport`] over a byte stream.
#[derive(Debug)]
pub struct LineStream<S> {
    stream: BufReader<S>,
    framing: Framing,
    name: String,
    partial: Vec<u8>,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> LineStream<S> {
    /// Lines over `stream`, framed by `framing`, traced as `name`.
    pub fn new(stream: S, framing: Framing, name: impl Into<String>) -> Self {
        Self {
            stream: BufReader::new(stream),
            framing,
            name: name.into(),
            partial: Vec::new(),
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> LineTransport for LineStream<S> {
    async fn write_line(&mut self, line: &str) -> Result<(), SerialError> {
        debug!(link = %self.name, tx = line);
        let stream = self.stream.get_mut();
        stream.write_all(line.as_bytes()).await?;
        stream.write_all(self.framing.write.as_bytes()).await?;
        Ok(stream.flush().await?)
    }

    async fn write_realtime(&mut self, byte: u8) -> Result<(), SerialError> {
        debug!(link = %self.name, tx_realtime = format_args!("{byte:#04x}"));
        let stream = self.stream.get_mut();
        stream.write_all(&[byte]).await?;
        Ok(stream.flush().await?)
    }

    async fn read_line(&mut self, timeout: Duration) -> Result<String, SerialError> {
        // `read_until` appends to `partial` as it reads, so a timeout keeps what arrived.
        let read = self.stream.read_until(self.framing.read, &mut self.partial);
        match tokio::time::timeout(timeout, read).await {
            Err(_) => Err(SerialError::Timeout),
            Ok(Err(error)) => Err(error.into()),
            Ok(Ok(0)) if self.partial.is_empty() => Err(SerialError::Closed),
            Ok(Ok(_)) => {
                let mut bytes = std::mem::take(&mut self.partial);
                while bytes
                    .last()
                    .is_some_and(|&b| b == self.framing.read || b == b'\r')
                {
                    bytes.pop();
                }
                let line = String::from_utf8_lossy(&bytes).into_owned();
                debug!(link = %self.name, rx = line);
                Ok(line)
            }
        }
    }
}

/// A link to an emulator in the same process.
pub type DuplexTransport = LineStream<DuplexStream>;

/// The host's end of an in-memory link, and the device's end for an emulator to serve.
pub fn duplex(framing: Framing, name: &str) -> (DuplexTransport, DuplexStream) {
    let (host, device) = tokio::io::duplex(4096);
    (LineStream::new(host, framing, name), device)
}

/// A serial port's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortSettings {
    /// The baud rate.
    pub baud: u32,
    /// The data bits: 5 to 8.
    pub data_bits: u8,
    /// The parity: `'N'`, `'E'` or `'O'`.
    pub parity: char,
    /// The stop bits: 1 or 2.
    pub stop_bits: u8,
    /// Whether RTS/CTS flow control is on.
    pub hardware_flow: bool,
}

impl Default for PortSettings {
    /// 115 200 baud, 8N1, no flow control: GRBL's.
    fn default() -> Self {
        Self {
            baud: 115_200,
            data_bits: 8,
            parity: 'N',
            stop_bits: 1,
            hardware_flow: false,
        }
    }
}

/// A serial port.
pub type SerialTransport = LineStream<SerialStream>;

/// Opens serial port `port`, such as `"COM5"`.
pub fn open(
    port: &str,
    settings: &PortSettings,
    framing: Framing,
) -> Result<SerialTransport, SerialError> {
    let invalid = |what: &str| {
        SerialError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{port}: {what}"),
        ))
    };
    let data_bits = match settings.data_bits {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        8 => DataBits::Eight,
        _ => return Err(invalid("data bits must be 5 to 8")),
    };
    let parity = match settings.parity.to_ascii_uppercase() {
        'N' => Parity::None,
        'E' => Parity::Even,
        'O' => Parity::Odd,
        _ => return Err(invalid("parity must be N, E or O")),
    };
    let stop_bits = match settings.stop_bits {
        1 => StopBits::One,
        2 => StopBits::Two,
        _ => return Err(invalid("stop bits must be 1 or 2")),
    };
    let flow = if settings.hardware_flow {
        FlowControl::Hardware
    } else {
        FlowControl::None
    };
    let stream = tokio_serial::new(port, settings.baud)
        .data_bits(data_bits)
        .parity(parity)
        .stop_bits(stop_bits)
        .flow_control(flow)
        .open_native_async()
        .map_err(|error| SerialError::Io(io::Error::other(format!("{port}: {error}"))))?;
    Ok(LineStream::new(stream, framing, port))
}

/// A serial port this computer has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    /// Its name, such as `COM5`.
    pub name: String,
    /// What it is: a USB adapter's maker, product and ids, or its kind.
    pub description: String,
}

/// The serial ports this computer has, by name.
pub fn ports() -> Result<Vec<Port>, SerialError> {
    let found = tokio_serial::available_ports().map_err(|error| SerialError::Io(error.into()))?;
    let mut ports: Vec<Port> = found
        .into_iter()
        .map(|port| Port {
            description: match port.port_type {
                SerialPortType::UsbPort(usb) => {
                    let named = [usb.manufacturer, usb.product].into_iter().flatten();
                    let named = named.collect::<Vec<_>>().join(" ");
                    format!("USB {:04x}:{:04x} {named}", usb.vid, usb.pid)
                        .trim_end()
                        .to_owned()
                }
                SerialPortType::PciPort => "PCI".to_owned(),
                SerialPortType::BluetoothPort => "Bluetooth".to_owned(),
                SerialPortType::Unknown => "unknown".to_owned(),
            },
            name: port.port_name,
        })
        .collect();
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ports)
}
