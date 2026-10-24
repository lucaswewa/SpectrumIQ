use std::time::Duration;

use super::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * (1.0 + a.abs().max(b.abs()))
}

#[test]
fn arithmetic_keeps_the_unit() {
    let (a, b) = (Micrometers(1.5), Micrometers(0.5));
    assert_eq!(a + b, Micrometers(2.0));
    assert_eq!(a - b, Micrometers(1.0));
    assert_eq!(-a, Micrometers(-1.5));
    assert_eq!(a * 2.0, Micrometers(3.0));
    assert_eq!(a / 3.0, Micrometers(0.5));
    assert_eq!(a / b, 3.0);
    let mut c = a;
    c += b;
    c -= Micrometers(1.0);
    assert_eq!(c, Micrometers(1.0));
    assert_eq!(Micrometers(-2.0).abs(), Micrometers(2.0));
    assert_eq!(Micrometers::ZERO.get(), 0.0);
}

#[test]
fn display_appends_the_symbol_and_honours_precision() {
    assert_eq!(Micrometers(1.5).to_string(), "1.5 µm");
    assert_eq!(format!("{:.2}", Millis(3.0)), "3.00 ms");
    assert_eq!(Hertz(9.0).to_string(), "9 Hz");
    assert_eq!(Nanometers(550.0).to_string(), "550 nm");
    assert_eq!(Millimeters::SYMBOL, "mm");
}

#[test]
fn lengths_convert_between_units() {
    assert_eq!(Micrometers::from(Millimeters(1.25)), Micrometers(1250.0));
    assert!(close(Millimeters::from(Micrometers(500.0)).0, 0.5));
    assert!(close(Micrometers::from(Nanometers(550.0)).0, 0.55));
    assert!(close(Nanometers::from(Micrometers(0.55)).0, 550.0));
}

#[test]
fn durations_and_rates_convert() {
    assert!(close(Millis::from(Duration::from_micros(1500)).0, 1.5));
    let duration = Millis(2.5).to_duration().expect("a valid duration");
    assert!(duration.as_nanos().abs_diff(2_500_000) <= 1);
    assert_eq!(Millis(-1.0).to_duration(), None);
    assert_eq!(Millis(f64::NAN).to_duration(), None);
    assert_eq!(Hertz(8.0).period(), Millis(125.0));
    assert_eq!(Hertz::from_period(Millis(125.0)), Hertz(8.0));
}

#[test]
fn units_serialize_as_bare_numbers() {
    assert_eq!(serde_json::to_string(&Micrometers(2.5)).unwrap(), "2.5");
    assert_eq!(serde_json::from_str::<Millis>("40").unwrap(), Millis(40.0));
}
