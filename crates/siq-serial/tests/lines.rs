//! Lines over the in-memory duplex: framing, partial lines across timeouts, realtime bytes, and
//! a closed link; and the ports listed.

use std::time::Duration;

use siq_serial::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SHORT: Duration = Duration::from_millis(50);

// REQ-STG-001
#[tokio::test]
async fn lines_and_realtime_bytes_cross_the_link_as_framed() {
    let (mut host, mut device) = duplex(Framing::default(), "test");
    host.write_line("G1 X10 F600").await.unwrap();
    host.write_realtime(b'?').await.unwrap();
    let mut sent = [0u8; 13];
    device.read_exact(&mut sent).await.unwrap();
    assert_eq!(&sent, b"G1 X10 F600\n?");

    // A reply split across writes, with \r\n, arrives whole; a timeout keeps what has come.
    device.write_all(b"<Idle|MPos:0.000").await.unwrap();
    assert!(matches!(
        host.read_line(SHORT).await,
        Err(SerialError::Timeout)
    ));
    device
        .write_all(b",0.000,0.000|FS:0,0>\r\nok\r\n")
        .await
        .unwrap();
    assert_eq!(
        host.read_line(SHORT).await.unwrap(),
        "<Idle|MPos:0.000,0.000,0.000|FS:0,0>"
    );
    assert_eq!(host.read_line(SHORT).await.unwrap(), "ok");

    drop(device);
    assert!(matches!(
        host.read_line(SHORT).await,
        Err(SerialError::Closed)
    ));
}

#[tokio::test]
async fn a_dialect_can_frame_its_lines_with_carriage_returns() {
    let framing = Framing {
        write: "\r".into(),
        read: b'\r',
    };
    let (mut host, mut device) = duplex(framing, "focus");
    host.write_line("MA 1200").await.unwrap();
    let mut sent = [0u8; 8];
    device.read_exact(&mut sent).await.unwrap();
    assert_eq!(&sent, b"MA 1200\r");
    device.write_all(b"POS 1200\rOK\r").await.unwrap();
    assert_eq!(host.read_line(SHORT).await.unwrap(), "POS 1200");
    assert_eq!(host.read_line(SHORT).await.unwrap(), "OK");
}

#[test]
fn a_port_that_isnt_there_is_an_error_naming_it() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    let error = open("COM250", &PortSettings::default(), Framing::default()).unwrap_err();
    assert!(error.to_string().contains("COM250"), "{error}");
    let bad = PortSettings {
        parity: 'X',
        ..PortSettings::default()
    };
    assert!(
        open("COM250", &bad, Framing::default())
            .unwrap_err()
            .to_string()
            .contains("parity")
    );
}

// P10.3: this computer's serial ports, by name, each told what it is.
#[test]
fn the_ports_are_listed_by_name() {
    let found = ports().expect("the ports can be listed");
    let names: Vec<&str> = found.iter().map(|port| port.name.as_str()).collect();
    assert!(names.is_sorted(), "{names:?}");
    assert!(
        found
            .iter()
            .all(|port| !port.name.is_empty() && !port.description.is_empty())
    );
}
