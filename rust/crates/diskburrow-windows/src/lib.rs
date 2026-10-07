//! Metadata never recalls cloud content. Deletion is single-use, reviewed, and handle-bound.
mod cleanup;
mod environment;
mod native;
mod paths;
pub use cleanup::*;
pub use diskburrow_services::{FileIdentity, FileObservation as NativeFileObservation};
pub use environment::{
    FixedRuleEnvironment, KnownDirectories, OwnerProcessState, RuleEnvironment,
    WindowsRuleEnvironment,
};
pub use native::{NativeFileApi, NativeHandle, WindowsNativeFileApi, inspect, is_cloud};
pub use paths::{equals_path, is_within, normalize_local_path};
