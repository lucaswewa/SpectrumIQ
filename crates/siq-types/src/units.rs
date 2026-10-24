//! Physical quantities as `f64` newtypes.
//!
//! Each unit adds and subtracts with itself, scales by `f64`, and divides by itself to give a
//! ratio. It is serialized as a bare number (`serde(transparent)`), and its JSON Schema says the
//! unit. `Display` appends the unit's symbol, and honours a precision: `{:.2}` gives `1.50 µm`.

#[cfg(test)]
mod tests;

use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! unit {
    ($(#[$doc:meta])* $name:ident, $symbol:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(pub f64);

        impl $name {
            /// Zero.
            pub const ZERO: Self = Self(0.0);
            /// The unit's symbol.
            pub const SYMBOL: &'static str = $symbol;

            /// The quantity's value, in this unit.
            pub const fn get(self) -> f64 {
                self.0
            }

            /// The magnitude.
            pub fn abs(self) -> Self {
                Self(self.0.abs())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match f.precision() {
                    Some(precision) => write!(f, "{:.*} {}", precision, self.0, $symbol),
                    None => write!(f, "{} {}", self.0, $symbol),
                }
            }
        }

        impl Add for $name {
            type Output = Self;
            fn add(self, other: Self) -> Self {
                Self(self.0 + other.0)
            }
        }

        impl Sub for $name {
            type Output = Self;
            fn sub(self, other: Self) -> Self {
                Self(self.0 - other.0)
            }
        }

        impl Neg for $name {
            type Output = Self;
            fn neg(self) -> Self {
                Self(-self.0)
            }
        }

        impl Mul<f64> for $name {
            type Output = Self;
            fn mul(self, factor: f64) -> Self {
                Self(self.0 * factor)
            }
        }

        impl Div<f64> for $name {
            type Output = Self;
            fn div(self, divisor: f64) -> Self {
                Self(self.0 / divisor)
            }
        }

        /// The ratio of two quantities of the same unit.
        impl Div for $name {
            type Output = f64;
            fn div(self, other: Self) -> f64 {
                self.0 / other.0
            }
        }

        impl AddAssign for $name {
            fn add_assign(&mut self, other: Self) {
                self.0 += other.0;
            }
        }

        impl SubAssign for $name {
            fn sub_assign(&mut self, other: Self) {
                self.0 -= other.0;
            }
        }
    };
}

unit!(
    /// A length in micrometres (µm): SpectrumIQ's unit for positions and distances.
    Micrometers,
    "µm"
);
unit!(
    /// A length in millimetres (mm), as G-code and some controllers use.
    Millimeters,
    "mm"
);
unit!(
    /// A length in nanometres (nm), for wavelengths and fine steps.
    Nanometers,
    "nm"
);
unit!(
    /// A duration in milliseconds (ms), such as an exposure time.
    Millis,
    "ms"
);
unit!(
    /// A frequency in hertz (Hz), such as a frame rate.
    Hertz,
    "Hz"
);
unit!(
    /// A speed in micrometres per second (µm/s), for the stage and the focus drive.
    MicrometersPerSecond,
    "µm/s"
);

impl From<Millimeters> for Micrometers {
    fn from(length: Millimeters) -> Self {
        Self(length.0 * 1e3)
    }
}

impl From<Nanometers> for Micrometers {
    fn from(length: Nanometers) -> Self {
        Self(length.0 * 1e-3)
    }
}

impl From<Micrometers> for Millimeters {
    fn from(length: Micrometers) -> Self {
        Self(length.0 * 1e-3)
    }
}

impl From<Micrometers> for Nanometers {
    fn from(length: Micrometers) -> Self {
        Self(length.0 * 1e3)
    }
}

impl From<Duration> for Millis {
    fn from(duration: Duration) -> Self {
        Self(duration.as_secs_f64() * 1e3)
    }
}

impl Millis {
    /// The duration, or `None` if it is negative or not finite.
    pub fn to_duration(self) -> Option<Duration> {
        Duration::try_from_secs_f64(self.0 * 1e-3).ok()
    }
}

impl Hertz {
    /// The time between two events at this rate.
    pub fn period(self) -> Millis {
        Millis(1e3 / self.0)
    }

    /// The rate of events `period` apart.
    pub fn from_period(period: Millis) -> Self {
        Self(1e3 / period.0)
    }
}
