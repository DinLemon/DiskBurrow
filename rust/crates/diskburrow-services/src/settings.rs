use crate::is_fully_qualified;
use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct AppSettings {
    pub interval_hours: i32,
    pub initial_delay: String,
    pub free_space_interval: String,
    pub low_space_bytes: i64,
    pub growth_bytes: i64,
    pub language: String,
    pub theme: String,
    pub ui_scale_percent: u16,
    pub map_depth: u8,
    pub show_hidden: bool,
    pub sidebar_width: u16,
    pub excluded_paths: Vec<String>,
    pub paused: bool,
    pub allow_on_battery: bool,
    pub autostart: bool,
    pub approved_custom_temp_path: Option<String>,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            interval_hours: 6,
            initial_delay: "00:05:00".into(),
            free_space_interval: "00:05:00".into(),
            low_space_bytes: 15_000_000_000,
            growth_bytes: 5_000_000_000,
            language: "ru".into(),
            theme: "light".into(),
            ui_scale_percent: 100,
            map_depth: 3,
            show_hidden: true,
            sidebar_width: 225,
            excluded_paths: vec![],
            paused: false,
            allow_on_battery: false,
            autostart: false,
            approved_custom_temp_path: None,
        }
    }
}
impl AppSettings {
    pub fn validate(&self) -> Result<()> {
        if ![1, 6, 12, 24].contains(&self.interval_hours)
            || self.low_space_bytes < 0
            || self.growth_bytes <= 0
            || self.initial_delay != "00:05:00"
            || self.free_space_interval != "00:05:00"
            || !["ru", "en"].contains(&self.language.as_str())
            || !["light", "dark", "system"].contains(&self.theme.as_str())
            || ![75, 90, 100, 110, 125, 150].contains(&self.ui_scale_percent)
            || !(1..=6).contains(&self.map_depth)
            || !(180..=420).contains(&self.sidebar_width)
            || self.excluded_paths.iter().any(|p| !is_fully_qualified(p))
            || self
                .approved_custom_temp_path
                .as_ref()
                .is_some_and(|p| !is_fully_qualified(p))
        {
            bail!("Invalid DiskBurrow settings.");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct AlertSuppressionState {
    pub low_space_roots: BTreeSet<String>,
    pub last_growth_notified_utc: BTreeMap<String, DateTime<Utc>>,
    pub last_growth_snapshot: BTreeMap<String, Uuid>,
    pub evaluated_growth_snapshots: BTreeMap<String, Vec<Uuid>>,
}
impl AlertSuppressionState {
    pub fn validate(&self) -> Result<()> {
        if self
            .low_space_roots
            .iter()
            .chain(self.last_growth_notified_utc.keys())
            .chain(self.last_growth_snapshot.keys())
            .chain(self.evaluated_growth_snapshots.keys())
            .any(|p| !is_fully_qualified(p))
            || self
                .evaluated_growth_snapshots
                .values()
                .any(|v| v.len() > 30)
        {
            bail!("Invalid alert suppression state.");
        }
        Ok(())
    }
}
pub struct SettingsStore {
    pub directory: PathBuf,
    pub last_user_message: Option<String>,
}
impl SettingsStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            last_user_message: None,
        }
    }
    pub fn load_cancellable(&mut self, token: &crate::CancellationToken) -> Result<AppSettings> {
        token.check()?;
        let settings = self.load();
        token.check()?;
        Ok(settings)
    }
    pub fn save_cancellable(
        &mut self,
        settings: &AppSettings,
        token: &crate::CancellationToken,
    ) -> Result<()> {
        token.check()?;
        settings.validate()?;
        self.save_document("settings.json", settings, token)
    }
    pub fn load_alert_state_cancellable(
        &mut self,
        token: &crate::CancellationToken,
    ) -> Result<AlertSuppressionState> {
        token.check()?;
        let state = self.load_alert_state();
        token.check()?;
        Ok(state)
    }
    pub fn save_alert_state_cancellable(
        &mut self,
        state: &AlertSuppressionState,
        token: &crate::CancellationToken,
    ) -> Result<()> {
        token.check()?;
        state.validate()?;
        self.save_document("alerts.json", state, token)
    }
    pub fn load(&mut self) -> AppSettings {
        self.load_document("settings.json", |document| {
            let mut settings: AppSettings = serde_json::from_value(document.clone())?;
            if document.get("LowSpaceBytes").is_none() {
                settings.low_space_bytes = 15_i64 << 30;
            }
            if document.get("GrowthBytes").is_none() {
                settings.growth_bytes = 5_i64 << 30;
            }
            settings.validate()?;
            Ok(settings)
        })
    }
    pub fn save(&mut self, settings: &AppSettings) -> Result<()> {
        self.save_cancellable(settings, &crate::CancellationToken::new())
    }
    pub fn load_alert_state(&mut self) -> AlertSuppressionState {
        self.load_document("alerts.json", |document| {
            let state: AlertSuppressionState = serde_json::from_value(document)?;
            state.validate()?;
            Ok(state)
        })
    }
    pub fn save_alert_state(&mut self, state: &AlertSuppressionState) -> Result<()> {
        self.save_alert_state_cancellable(state, &crate::CancellationToken::new())
    }
    fn load_document<T: Default>(
        &mut self,
        name: &str,
        parse: impl FnOnce(serde_json::Value) -> Result<T>,
    ) -> T {
        let result = (|| {
            let bytes = fs::read(self.directory.join(name))?;
            parse(serde_json::from_slice(&bytes)?)
        })();
        match result {
            Ok(value) => {
                self.last_user_message = None;
                value
            }
            Err(error) => {
                self.last_user_message = if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                {
                    None
                } else {
                    Some(format!("{name}: {error}"))
                };
                T::default()
            }
        }
    }
    fn save_document(
        &mut self,
        name: &str,
        value: &impl Serialize,
        token: &crate::CancellationToken,
    ) -> Result<()> {
        let gate = crate::storage::directory_gate(&self.directory);
        let _guard = crate::storage::lock_cancellable(&gate, token)?;
        let temporary = self
            .directory
            .join(format!(".{name}.{}.tmp", Uuid::new_v4().simple()));
        let result = (|| {
            fs::create_dir_all(&self.directory)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let bytes = serde_json::to_vec(value)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            token.check()?;
            fs::rename(&temporary, self.directory.join(name))?;
            Ok(())
        })();
        if temporary.exists() {
            let _ = fs::remove_file(&temporary);
        }
        if !result
            .as_ref()
            .err()
            .is_some_and(|e: &anyhow::Error| e.is::<crate::Cancelled>())
        {
            self.last_user_message = result
                .as_ref()
                .err()
                .map(|e: &anyhow::Error| format!("{name}: {e}"));
        }
        result
    }
}
