use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! numeric_enum {
    ($name:ident { $($variant:ident = $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(i32)]
        pub enum $name { $($variant = $value),+ }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_i32(*self as i32) }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                match i32::deserialize(d)? { $($value => Ok(Self::$variant),)+ _ => Err(serde::de::Error::custom("Unknown enum value")) }
            }
        }
    };
}
numeric_enum!(ScanIssueKind { AccessDenied=0, ReparseSkipped=1, CloudSkipped=2, MetadataUnavailable=3, ChangedDuringScan=4, IoFailure=5 });
numeric_enum!(CleanupOutcome { Deleted=0, Missing=1, SkippedChanged=2, SkippedBusy=3, SkippedPolicy=4, Failed=5 });

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FileIdentity {
    pub volume: u64,
    pub file_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FileObservation {
    pub path: String,
    pub identity: Option<FileIdentity>,
    pub logical_bytes: i64,
    pub allocated_bytes: Option<i64>,
    pub modified_utc: DateTime<Utc>,
    pub link_count: i32,
    pub attributes: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DirectoryObservation {
    pub path: String,
    pub logical_bytes: i64,
    pub allocated_bytes: Option<i64>,
    pub coverage_complete: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ScanIssue {
    pub path: String,
    pub kind: ScanIssueKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ScanSnapshot {
    pub id: Uuid,
    pub root: String,
    pub started_utc: DateTime<Utc>,
    pub completed_utc: DateTime<Utc>,
    pub traversal_completed: bool,
    pub directories: Vec<DirectoryObservation>,
    pub largest_files: Vec<FileObservation>,
    pub issues: Vec<ScanIssue>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ScanProgress {
    pub files_visited: i64,
    pub directories_visited: i64,
    pub current_path: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VolumeSpace {
    pub total_bytes: i64,
    pub free_bytes: i64,
}
impl VolumeSpace {
    pub fn used_bytes(&self) -> i64 {
        self.total_bytes.saturating_sub(self.free_bytes)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VolumeObservation {
    pub root: String,
    pub space: VolumeSpace,
    pub observed_utc: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CleanupAudit {
    pub path: String,
    pub rule_id: String,
    pub rule_root_path: String,
    pub reviewed_modified_utc: DateTime<Utc>,
    pub logical_bytes: i64,
    pub allocated_bytes: Option<i64>,
    pub link_count: i32,
    pub identity: Option<FileIdentity>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CleanupItemResult {
    pub candidate_id: Uuid,
    pub outcome: CleanupOutcome,
    pub reason_key: Option<String>,
    #[serde(default)]
    pub audit: Option<CleanupAudit>,
}
fn default_true() -> bool {
    true
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CleanupReport {
    pub plan_id: Uuid,
    pub items: Vec<CleanupItemResult>,
    pub free_space_delta_bytes: i64,
    #[serde(default)]
    pub was_cancelled: bool,
    #[serde(default = "default_true")]
    pub free_space_delta_available: bool,
}

/// Windows paths are persistence identities; normalization is lexical and never follows links.
pub fn normalize_path(path: &str) -> String {
    let path = path.replace('/', "\\");
    let prefix_len = if path.starts_with("\\\\") {
        let mut separators = path.match_indices('\\').filter(|(i, _)| *i >= 2);
        separators.next();
        separators.next().map_or(path.len(), |(i, _)| i)
    } else if path.as_bytes().get(1) == Some(&b':') && path.as_bytes().get(2) == Some(&b'\\') {
        3
    } else {
        0
    };
    let (prefix, rest) = path.split_at(prefix_len);
    let mut segments: Vec<&str> = Vec::new();
    for part in rest.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(part),
        }
    }
    if segments.is_empty() {
        return prefix.to_owned();
    }
    format!(
        "{}{}{}",
        prefix,
        if prefix.ends_with('\\') || prefix.is_empty() {
            ""
        } else {
            "\\"
        },
        segments.join("\\")
    )
}
pub fn path_key(path: &str) -> String {
    normalize_path(path).to_uppercase()
}
pub fn is_fully_qualified(path: &str) -> bool {
    let b = path.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || (path.starts_with("\\\\")
            && path
                .trim_start_matches('\\')
                .split('\\')
                .filter(|p| !p.is_empty())
                .count()
                >= 2)
}
