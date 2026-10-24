//! GRBL's `$` settings: the ones the emulator keeps, with GRBL 1.1's numbers and defaults for a
//! microscope stage.

/// The settings, in GRBL's order, with their default values.
const DEFAULTS: [(u16, f64); 34] = [
    (0, 10.0),     // step pulse, µs
    (1, 25.0),     // step idle delay, ms
    (2, 0.0),      // step port invert
    (3, 0.0),      // direction port invert
    (4, 0.0),      // step enable invert
    (5, 0.0),      // limit pins invert
    (6, 0.0),      // probe pin invert
    (10, 1.0),     // status report: machine position
    (11, 0.010),   // junction deviation, mm
    (12, 0.002),   // arc tolerance, mm
    (13, 0.0),     // report in inches
    (20, 1.0),     // soft limits
    (21, 0.0),     // hard limits
    (22, 1.0),     // homing cycle
    (23, 0.0),     // homing direction invert
    (24, 25.0),    // homing feed, mm/min
    (25, 1000.0),  // homing seek, mm/min
    (26, 250.0),   // homing debounce, ms
    (27, 1.0),     // homing pull-off, mm
    (30, 1000.0),  // maximum spindle speed
    (31, 0.0),     // minimum spindle speed
    (32, 0.0),     // laser mode
    (100, 1600.0), // x steps/mm
    (101, 1600.0), // y steps/mm
    (102, 3200.0), // z steps/mm
    (110, 3000.0), // x maximum rate, mm/min
    (111, 3000.0), // y maximum rate, mm/min
    (112, 600.0),  // z maximum rate, mm/min
    (120, 200.0),  // x acceleration, mm/s²
    (121, 200.0),  // y acceleration, mm/s²
    (122, 100.0),  // z acceleration, mm/s²
    (130, 100.0),  // x maximum travel, mm
    (131, 75.0),   // y maximum travel, mm
    (132, 20.0),   // z maximum travel, mm
];

/// The settings that GRBL prints with three decimals.
const DECIMAL: [u16; 17] = [
    11, 12, 24, 25, 27, 100, 101, 102, 110, 111, 112, 120, 121, 122, 130, 131, 132,
];

/// The emulator's `$` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct GrblSettings {
    values: Vec<(u16, f64)>,
}

impl Default for GrblSettings {
    /// A stage of 100 × 75 × 20 mm, at 3000 mm/min and 200 mm/s² in XY, with soft limits and
    /// homing on.
    fn default() -> Self {
        Self {
            values: DEFAULTS.to_vec(),
        }
    }
}

impl GrblSettings {
    /// Setting `$number`, if the emulator has it.
    pub fn get(&self, number: u16) -> Option<f64> {
        self.values
            .iter()
            .find(|(n, _)| *n == number)
            .map(|&(_, value)| value)
    }

    /// Sets `$number`, and says whether the emulator has it.
    pub fn set(&mut self, number: u16, value: f64) -> bool {
        let slot = self.values.iter_mut().find(|(n, _)| *n == number);
        slot.map(|(_, v)| *v = value).is_some()
    }

    /// The `$$` listing, as GRBL prints it.
    pub(crate) fn listing(&self) -> impl Iterator<Item = String> + '_ {
        self.values
            .iter()
            .map(|&(number, value)| match DECIMAL.contains(&number) {
                true => format!("${number}={value:.3}"),
                false => format!("${number}={value:.0}"),
            })
    }

    /// Setting `base + axis`, which the emulator always has.
    pub(crate) fn axis(&self, base: u16, axis: usize) -> f64 {
        self.get(base + axis as u16).expect("an axis setting")
    }

    /// Setting `number` as a switch.
    pub(crate) fn on(&self, number: u16) -> bool {
        self.get(number).is_some_and(|value| value != 0.0)
    }
}
