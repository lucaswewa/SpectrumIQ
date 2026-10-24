//! GRBL's replies: status reports, settings, and what errors and alarms mean.

use siq_devices::DeviceError;

/// A controller's state, from a status report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Still, and ready.
    Idle,
    /// Running G-code.
    Run,
    /// Stopped in a feed hold.
    Hold,
    /// Jogging.
    Jog,
    /// In alarm, or locked until homed.
    Alarm,
    /// Stopped with a safety door open.
    Door,
    /// Checking G-code without moving.
    Check,
    /// Homing.
    Home,
    /// Asleep.
    Sleep,
}

/// A status report: `<Idle|MPos:1.000,2.000,3.000|FS:0,0>`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Report {
    /// The state.
    pub state: State,
    /// The machine position of x, y and z, in mm.
    pub position: [f64; 3],
    /// The feed rate, in mm/min.
    pub feed: f64,
}

/// Parses a status report, or gives `None` for any other line.
///
/// - A work position (`WPos`) is made a machine one with the work offset. GRBL sends the offset
///   (`WCO`) only now and then, and after a reset, so the last one is kept in `offset`.
/// - A hold or a door still stopping (`Hold:1`, `Door:2`, `Door:3`) is `Run`: the axes are still
///   moving, and a reset then would lose the position.
/// - Fields it doesn't need, and grblHAL's extra axes, are ignored.
pub fn parse_status(line: &str, offset: &mut [f64; 3]) -> Option<Report> {
    let mut fields = line.strip_prefix('<')?.strip_suffix('>')?.split('|');
    let head = fields.next()?;
    let state = match head.split_once(':').unwrap_or((head, "")) {
        ("Hold", "1") | ("Door", "2" | "3") => State::Run,
        ("Idle", _) => State::Idle,
        ("Run", _) => State::Run,
        ("Hold" | "Tool", _) => State::Hold,
        ("Jog", _) => State::Jog,
        ("Alarm", _) => State::Alarm,
        ("Door", _) => State::Door,
        ("Check", _) => State::Check,
        ("Home", _) => State::Home,
        ("Sleep", _) => State::Sleep,
        _ => return None,
    };
    let (mut machine, mut work, mut wco, mut feed) = (None, None, None, 0.0);
    for field in fields {
        let (name, value) = field.split_once(':')?;
        match name {
            "MPos" => machine = Some(axes(value)?),
            "WPos" => work = Some(axes(value)?),
            "WCO" => wco = Some(axes(value)?),
            "FS" | "F" => feed = value.split(',').next()?.parse().ok()?,
            _ => {}
        }
    }
    let wco = wco.unwrap_or(*offset);
    let position = machine.or_else(|| work.map(|w| std::array::from_fn(|i| w[i] + wco[i])))?;
    *offset = wco;
    Some(Report {
        state,
        position,
        feed,
    })
}

/// The first three of a field's comma-separated numbers.
fn axes(value: &str) -> Option<[f64; 3]> {
    let mut numbers = value.split(',').map(|n| n.parse::<f64>().ok());
    let mut next = || numbers.next().flatten().filter(|n| n.is_finite());
    Some([next()?, next()?, next()?])
}

/// Parses a line of `$$`: `$110=3000.000` is setting 110.
pub fn parse_setting(line: &str) -> Option<(u16, f64)> {
    let (number, value) = line.strip_prefix('$')?.split_once('=')?;
    let value = value
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())?;
    Some((number.parse().ok()?, value))
}

/// The device error for GRBL's `error:code`.
pub fn error(code: u32) -> DeviceError {
    let what = match code {
        1 => "a word without a letter",
        2 => "a bad number",
        3 => "an invalid $ statement",
        5 => "homing isn't enabled",
        9 => "locked out by an alarm or a jog",
        15 => "a jog past the travel",
        20 => "an unsupported command",
        22 => "no feed rate",
        _ => "see GRBL's documentation",
    };
    let message = format!("GRBL error {code}: {what}");
    match code {
        2 | 15 | 20 | 22 => DeviceError::InvalidArgument(message),
        9 => DeviceError::Busy(message),
        _ => DeviceError::Fault(message),
    }
}

/// Whether alarm `code` loses the machine position, so that homing is needed: a hard limit (1),
/// a reset in motion (3) or a homing failure (6 to 9).
pub fn loses_position(code: u32) -> bool {
    matches!(code, 1 | 3 | 6..=9)
}
