//! GRBL's replies, parsed: status reports as GRBL 1.1 and grblHAL print them, `$$` settings, and
//! lines that are neither.

use proptest::prelude::*;
use siq_devices::DeviceError;
use siq_grbl::protocol::{self, Report, State};

/// Every state, with the substates GRBL may print after it, and what it parses as.
const STATES: [(&str, State); 14] = [
    ("Idle", State::Idle),
    ("Run", State::Run),
    ("Hold:0", State::Hold),
    ("Hold:1", State::Run),
    ("Jog", State::Jog),
    ("Alarm", State::Alarm),
    ("Door:0", State::Door),
    ("Door:1", State::Door),
    ("Door:2", State::Run),
    ("Door:3", State::Run),
    ("Check", State::Check),
    ("Home", State::Home),
    ("Sleep", State::Sleep),
    ("Tool", State::Hold),
];

fn millimetres() -> impl Strategy<Value = f64> {
    (-200_000i32..=200_000).prop_map(|steps| f64::from(steps) / 1000.0)
}

proptest! {
    #[test]
    fn a_machine_position_report_parses_as_printed(
        state in 0..STATES.len(),
        [x, y, z] in [millimetres(), millimetres(), millimetres()],
        feed in 0u32..10_000,
        extra in prop::sample::select(vec!["", "|Bf:15,128", "|Pn:XZ|Ov:100,100,100", "|Ln:42|A:S"]),
    ) {
        let (name, parsed) = STATES[state];
        let line = format!("<{name}|MPos:{x:.3},{y:.3},{z:.3}|FS:{feed},0{extra}>");
        let report = protocol::parse_status(&line, &mut [0.0; 3]).unwrap();
        prop_assert_eq!(report.state, parsed);
        prop_assert_eq!(report.feed, f64::from(feed));
        for (got, want) in report.position.iter().zip([x, y, z]) {
            prop_assert!((got - want).abs() < 1e-9, "{line}: {got} for {want}");
        }
    }

    #[test]
    fn a_work_position_parses_as_the_machine_position_by_the_last_offset(
        work in [millimetres(), millimetres(), millimetres()],
        offset in [millimetres(), millimetres(), millimetres()],
        later in [millimetres(), millimetres(), millimetres()],
        fourth in millimetres(),
    ) {
        // grblHAL may report a fourth axis, which the stage doesn't have; and GRBL sends the
        // offset only now and then.
        let ([x, y, z], [i, j, k], [u, v, w]) = (work, offset, later);
        let first = format!("<Idle|WPos:{x:.3},{y:.3},{z:.3},{fourth:.3}|F:0|WCO:{i:.3},{j:.3},{k:.3},0.000>");
        let second = format!("<Run|WPos:{u:.3},{v:.3},{w:.3}|FS:0,0>");
        let mut kept = [0.0; 3];
        for (line, work) in [(first, work), (second, later)] {
            let report = protocol::parse_status(&line, &mut kept).unwrap();
            for axis in 0..3 {
                let want = work[axis] + offset[axis];
                prop_assert!((report.position[axis] - want).abs() < 1e-9, "{line}");
            }
        }
    }

    #[test]
    fn a_line_that_isnt_a_report_parses_as_none(line in "[^<]\\PC*|<\\PC*[^>]|<[A-Za-z]{0,6}\\|?>") {
        prop_assert_eq!(protocol::parse_status(&line, &mut [0.0; 3]), None);
    }

    #[test]
    fn any_line_parses_without_panicking(line in "<?[A-Za-z:|,.0-9-]{0,60}>?") {
        let _ = protocol::parse_status(&line, &mut [0.0; 3]);
        let _ = protocol::parse_setting(&line);
    }

    #[test]
    fn a_setting_parses_as_printed(number in 0u16..1000, value in -1_000_000i64..1_000_000) {
        let value = value as f64 / 1000.0;
        let line = format!("${number}={value:.3}");
        prop_assert_eq!(protocol::parse_setting(&line), Some((number, value)));
    }
}

#[test]
fn reports_with_broken_fields_parse_as_none() {
    for line in [
        "<Idle>",
        "<Idle|FS:0,0>",
        "<Idle|MPos:1.000,2.000|FS:0,0>",
        "<Idle|MPos:1.000,x,3.000>",
        "<Idle|MPos:1.000,2.000,NaN>",
        "<Running|MPos:1.000,2.000,3.000>",
        "<Idle|MPos:1.000,2.000,3.000|FS:fast,0>",
        "Idle|MPos:1.000,2.000,3.000",
    ] {
        assert_eq!(protocol::parse_status(line, &mut [0.0; 3]), None, "{line}");
    }
    // A broken report's offset isn't kept.
    let mut kept = [0.0; 3];
    assert_eq!(protocol::parse_status("<Idle|WCO:1,2,3>", &mut kept), None);
    assert_eq!(kept, [0.0; 3]);
    let report = protocol::parse_status("<Idle|MPos:1.5,-2,3e0>", &mut kept);
    let expected = Report {
        state: State::Idle,
        position: [1.5, -2.0, 3.0],
        feed: 0.0,
    };
    assert_eq!(report, Some(expected));
}

#[test]
fn settings_lines_with_comments_or_without_values_parse_as_expected() {
    assert_eq!(
        protocol::parse_setting("$130=100.000 (x max travel, mm)"),
        Some((130, 100.0))
    );
    for line in [
        "$130=",
        "$x=1",
        "130=1",
        "$130",
        "ok",
        "[MSG:Caution: Unlocked]",
    ] {
        assert_eq!(protocol::parse_setting(line), None, "{line}");
    }
}

#[test]
fn errors_and_alarms_mean_what_grbl_says() {
    let error = protocol::error(20);
    assert!(matches!(&error, DeviceError::InvalidArgument(m) if m.contains("GRBL error 20")));
    assert!(matches!(protocol::error(9), DeviceError::Busy(_)));
    assert!(matches!(
        protocol::error(15),
        DeviceError::InvalidArgument(_)
    ));
    assert!(matches!(protocol::error(70), DeviceError::Fault(_)));
    let lost: Vec<u32> = (1..=10)
        .filter(|&code| protocol::loses_position(code))
        .collect();
    assert_eq!(lost, [1, 3, 6, 7, 8, 9]);
}
