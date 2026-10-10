use chrono::{DateTime, Utc};

/// Transient display options. Projection never alters retained scan metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapProjection {
    pub depth: u32,
    pub show_hidden: bool,
}

impl Default for MapProjection {
    fn default() -> Self {
        Self {
            depth: 3,
            show_hidden: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeBand {
    Unknown,
    Within7Days,
    Within30Days,
    Within180Days,
    Within365Days,
    Older,
}

impl AgeBand {
    /// Classify Unix-second last-write metadata; zero is the scan's unknown sentinel.
    pub fn from_modified(modified: i64, now: i64) -> Self {
        if modified <= 0
            || now <= 0
            || modified > now
            || DateTime::<Utc>::from_timestamp(modified, 0).is_none()
            || DateTime::<Utc>::from_timestamp(now, 0).is_none()
        {
            return Self::Unknown;
        }
        match now - modified {
            0..=604_800 => Self::Within7Days,
            604_801..=2_592_000 => Self::Within30Days,
            2_592_001..=15_552_000 => Self::Within180Days,
            15_552_001..=31_536_000 => Self::Within365Days,
            _ => Self::Older,
        }
    }
}
