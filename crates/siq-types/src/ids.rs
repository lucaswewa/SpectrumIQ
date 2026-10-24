//! Ids for datasets, calibrations, channels and objectives.
//!
//! - **Datasets and calibrations** are made at run time, so their ids are UUID v7: unique, and
//!   ordered by creation time.
//! - **Channels and objectives** are defined in the instrument configuration, which gives each one
//!   a number (`[[channels]] id = 3`). The id is that number, not the entry's position, so
//!   reordering the configuration keeps it. Datasets keep a snapshot of the configuration that
//!   resolves it.
//!
//! Every id is `Copy`, so a frame's metadata holds them without allocating.

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// A new id, ordered after every id this process made before it.
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }

            /// The id with this UUID.
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// The UUID.
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// The last 8 hex digits, for folder names and logs. They are random in a UUID v7;
            /// the first ones are its timestamp, and repeat for ids made within a minute.
            pub fn short(&self) -> String {
                self.0.simple().to_string()[24..].to_owned()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.hyphenated().fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(text).map(Self)
            }
        }
    };
}

macro_rules! config_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(pub u16);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

uuid_id!(
    /// A dataset: a scan, a stack, a snapshot, and everything derived from it.
    DatasetId
);
uuid_id!(
    /// A stage–camera calibration. Each recalibration gets a new id, and frames record the one in
    /// force when they were taken.
    CalibrationId
);
config_id!(
    /// A channel of the instrument configuration: illumination, filter, exposure and display
    /// colour.
    ChannelId
);
config_id!(
    /// An objective of the instrument configuration.
    ObjectiveId
);
