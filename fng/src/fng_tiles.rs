use hiarc::Hiarc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Hiarc, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Serialize, Deserialize)]
pub enum FngSpikeTiles {
    Golden = 7,
    Normal,
    Red,
    Blue,
    Green = 14,
    Purple,
}
