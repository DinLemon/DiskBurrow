use crate::*;
use anyhow::{Result, bail};
use rusqlite::{Connection, OpenFlags, params};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};
type DirectoryGates = HashMap<String, Weak<Mutex<()>>>;
static GATES: OnceLock<Mutex<DirectoryGates>> = OnceLock::new();
pub(crate) fn directory_gate(path: &Path) -> Arc<Mutex<()>> {
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let key = path_key(&full.to_string_lossy());
    let mut gates = GATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(gate) = gates.get(&key).and_then(Weak::upgrade) {
        return gate;
    }
    let gate = Arc::new(Mutex::new(()));
    gates.insert(key, Arc::downgrade(&gate));
    gate
}
#[derive(Debug)]
enum StorageFailure {
    Limit,
    Incompatible,
}
impl std::fmt::Display for StorageFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for StorageFailure {}
const SCHEMA_RESERVE: i64 = 32 * 1024;
#[derive(Clone)]
pub struct StorageBudget {
    pub directory_path: PathBuf,
    pub total_limit_bytes: i64,
    pub log_limit_bytes: i64,
    gate: Arc<Mutex<()>>,
}
impl StorageBudget {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self::with_limits(directory, 250 * 1024 * 1024, 10 * 1024 * 1024)
            .expect("Valid default storage budget")
    }
    pub fn with_limits(directory: impl Into<PathBuf>, total: i64, logs: i64) -> Result<Self> {
        if total <= 0 || logs < 0 {
            bail!("Invalid storage budget.");
        }
        let directory_path = std::path::absolute(directory.into())?;
        Ok(Self {
            gate: directory_gate(&directory_path),
            directory_path,
            total_limit_bytes: total,
            log_limit_bytes: logs.min(total),
        })
    }
    fn files(&self) -> Result<Vec<(PathBuf, i64)>> {
        fn visit(path: &Path, out: &mut Vec<(PathBuf, i64)>) -> Result<()> {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let metadata = entry.metadata()?;
                if metadata.is_dir() {
                    visit(&entry.path(), out)?;
                } else {
                    out.push((entry.path(), i64::try_from(metadata.len())?));
                }
            }
            Ok(())
        }
        let mut out = vec![];
        if self.directory_path.exists() {
            visit(&self.directory_path, &mut out)?;
        }
        Ok(out)
    }
    pub fn used_bytes(&self) -> Result<i64> {
        self.files()?.iter().try_fold(0_i64, |sum, (_, n)| {
            sum.checked_add(*n)
                .ok_or_else(|| anyhow::anyhow!("Storage size overflow"))
        })
    }
    pub fn database_capacity(&self, database: &Path, page_size: i64) -> Result<i64> {
        if page_size <= 0 {
            bail!("Invalid database page size.");
        }
        let files = self.files()?;
        let mut others = 0_i64;
        let mut logs = 0_i64;
        let database = path_key(&database.to_string_lossy());
        for (path, bytes) in files {
            if path_key(&path.to_string_lossy()) != database {
                others = others.saturating_add(bytes);
            }
            if path
                .extension()
                .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("log"))
            {
                logs = logs.saturating_add(bytes);
            }
        }
        let available = self
            .total_limit_bytes
            .saturating_sub(others)
            .saturating_sub((self.log_limit_bytes - logs).max(0))
            .saturating_sub(64 * 1024)
            .max(0);
        Ok(available / (2 * page_size + 8) * page_size)
    }
    pub fn append_log(&self, message: &str) -> bool {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        (|| -> Result<bool> {
            let bytes = format!("{message}\r\n").into_bytes();
            let path = self.directory_path.join("diskburrow.log");
            let files = self.files()?;
            let current = files
                .iter()
                .find(|(p, _)| path_key(&p.to_string_lossy()) == path_key(&path.to_string_lossy()))
                .map_or(0, |(_, n)| *n);
            let total = files.iter().fold(0_i64, |n, (_, b)| n.saturating_add(*b));
            let other_logs = files
                .iter()
                .filter(|(p, _)| {
                    p != &path
                        && p.extension()
                            .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("log"))
                })
                .fold(0_i64, |n, (_, b)| n.saturating_add(*b));
            let allowance =
                (self.log_limit_bytes - other_logs).min(self.total_limit_bytes - total + current);
            if i64::try_from(bytes.len())? > allowance {
                return Ok(false);
            }
            fs::create_dir_all(&self.directory_path)?;
            let mut stream = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path)?;
            if i64::try_from(stream.metadata()?.len())?.saturating_add(bytes.len() as i64)
                > allowance
            {
                stream.set_len(0)?;
            }
            stream.seek(SeekFrom::End(0))?;
            stream.write_all(&bytes)?;
            stream.sync_all()?;
            Ok(true)
        })()
        .unwrap_or(false)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryWriteResult {
    pub saved: bool,
    pub user_message: Option<String>,
}
pub struct SqliteHistoryStore {
    pub budget: StorageBudget,
    pub database_path: PathBuf,
    pub last_user_message: Option<String>,
    pub last_successful_snapshot: Option<ScanSnapshot>,
    pub last_completed_traversal: Option<ScanSnapshot>,
}
impl SqliteHistoryStore {
    pub fn new(budget: StorageBudget) -> Self {
        Self {
            database_path: budget.directory_path.join("history.db"),
            budget,
            last_user_message: None,
            last_successful_snapshot: None,
            last_completed_traversal: None,
        }
    }
    pub fn save(&mut self, snapshot: &ScanSnapshot) -> HistoryWriteResult {
        self.save_cancellable(snapshot, &CancellationToken::new())
            .expect("A fresh token cannot cancel")
    }
    pub fn save_cancellable(
        &mut self,
        snapshot: &ScanSnapshot,
        token: &CancellationToken,
    ) -> Result<HistoryWriteResult> {
        token.check()?;
        let gate = self.budget.gate.clone();
        let _guard = lock_cancellable(&gate, token)?;
        if !snapshot.traversal_completed {
            return Ok(self.failure("Only completed traversals can be saved.".into()));
        }
        let result = (|| -> Result<()> {
            let mut stored = snapshot.clone();
            stored
                .largest_files
                .sort_by_key(|file| std::cmp::Reverse(file.logical_bytes));
            stored.largest_files.truncate(100);
            let payload = serde_json::to_vec(&stored)?;
            token.check()?;
            self.check_capacity(payload.len() as i64)?;
            let mut db = self.open(true)?;
            let page_size = scalar(&db, "PRAGMA page_size")?;
            let capacity = self
                .budget
                .database_capacity(&self.database_path, page_size)?;
            let transaction = db.transaction()?;
            transaction.execute(
                "DELETE FROM snapshots WHERE id=?1",
                [snapshot.id.to_string()],
            )?;
            transaction.execute("DELETE FROM snapshots WHERE id IN (SELECT id FROM snapshots ORDER BY completed DESC,rowid DESC LIMIT -1 OFFSET 29)",[])?;
            make_room(&transaction, payload.len() as i64, capacity, page_size)?;
            token.check()?;
            transaction.execute(
                "INSERT INTO snapshots(id,root,completed,payload) VALUES(?1,?2,?3,?4)",
                params![
                    snapshot.id.to_string(),
                    path_key(&snapshot.root),
                    dotnet_utc_ticks(snapshot.completed_utc)?,
                    payload
                ],
            )?;
            token.check()?;
            transaction.commit()?;
            // Once committed, optional vacuum cannot turn the saved traversal into a failed write.
            self.last_successful_snapshot = Some(snapshot.clone());
            self.last_completed_traversal = Some(snapshot.clone());
            let _ = db.execute_batch("PRAGMA incremental_vacuum(64)");
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.last_user_message = None;
                Ok(HistoryWriteResult {
                    saved: true,
                    user_message: None,
                })
            }
            Err(e) if e.is::<Cancelled>() => Err(e),
            Err(e) => Ok(self.failure(storage_message(&e))),
        }
    }
    pub fn load_recent(&mut self, root: &str, limit: usize) -> Vec<ScanSnapshot> {
        self.load_recent_cancellable(root, limit, &CancellationToken::new())
            .expect("A fresh token cannot cancel")
    }
    pub fn load_recent_cancellable(
        &mut self,
        root: &str,
        limit: usize,
        token: &CancellationToken,
    ) -> Result<Vec<ScanSnapshot>> {
        token.check()?;
        let gate = self.budget.gate.clone();
        let _guard = lock_cancellable(&gate, token)?;
        if limit == 0 || !self.database_path.exists() {
            return Ok(vec![]);
        }
        let result = (|| -> Result<Vec<ScanSnapshot>> {
            let db = self.open(false)?;
            let mut query=db.prepare("SELECT payload FROM snapshots WHERE root=?1 ORDER BY completed DESC,rowid DESC LIMIT ?2")?;
            let rows = query.query_map(params![path_key(root), limit.min(30) as i64], |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
            let mut snapshots = vec![];
            for row in rows {
                token.check()?;
                let snapshot: ScanSnapshot = serde_json::from_slice(&row?)?;
                if !snapshot.traversal_completed || snapshot.largest_files.len() > 100 {
                    return Err(StorageFailure::Incompatible.into());
                }
                snapshots.push(snapshot);
            }
            Ok(snapshots)
        })();
        match result {
            Ok(rows) => {
                self.last_user_message = None;
                Ok(rows)
            }
            Err(e) if e.is::<Cancelled>() => Err(e),
            Err(e) => {
                self.last_user_message = Some(storage_message(&e));
                Ok(vec![])
            }
        }
    }
    pub fn append_cleanup(&mut self, report: &CleanupReport) -> HistoryWriteResult {
        self.append_cleanup_cancellable(report, &CancellationToken::new())
            .expect("A fresh token cannot cancel")
    }
    pub fn append_cleanup_cancellable(
        &mut self,
        report: &CleanupReport,
        token: &CancellationToken,
    ) -> Result<HistoryWriteResult> {
        token.check()?;
        let gate = self.budget.gate.clone();
        let _guard = lock_cancellable(&gate, token)?;
        let result = (|| -> Result<()> {
            let payload = serde_json::to_vec(report)?;
            self.check_capacity(payload.len() as i64)?;
            let mut db = self.open(true)?;
            let page_size = scalar(&db, "PRAGMA page_size")?;
            let transaction = db.transaction()?;
            transaction.execute(
                "DELETE FROM cleanup_reports WHERE id=?1",
                [report.plan_id.to_string()],
            )?;
            transaction.execute("DELETE FROM cleanup_reports WHERE id IN (SELECT id FROM cleanup_reports ORDER BY created DESC,rowid DESC LIMIT -1 OFFSET 99)",[])?;
            make_room(
                &transaction,
                payload.len() as i64,
                self.budget
                    .database_capacity(&self.database_path, page_size)?,
                page_size,
            )?;
            token.check()?;
            transaction.execute(
                "INSERT INTO cleanup_reports(id,created,payload) VALUES(?1,?2,?3)",
                params![
                    report.plan_id.to_string(),
                    dotnet_utc_ticks(chrono::Utc::now())?,
                    payload
                ],
            )?;
            token.check()?;
            transaction.commit()?;
            let _ = db.execute_batch("PRAGMA incremental_vacuum(64)");
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.last_user_message = None;
                Ok(HistoryWriteResult {
                    saved: true,
                    user_message: None,
                })
            }
            Err(e) if e.is::<Cancelled>() => Err(e),
            Err(e) => Ok(self.failure(storage_message(&e))),
        }
    }
    fn check_capacity(&self, payload_bytes: i64) -> Result<()> {
        let capacity = self.budget.database_capacity(&self.database_path, 4096)?;
        if payload_bytes > capacity - SCHEMA_RESERVE
            || (self.database_path.exists()
                && i64::try_from(fs::metadata(&self.database_path)?.len())? > capacity)
        {
            return Err(StorageFailure::Limit.into());
        }
        Ok(())
    }
    fn open(&self, write: bool) -> Result<Connection> {
        if write {
            self.check_capacity(0)?;
            fs::create_dir_all(&self.budget.directory_path)?;
        }
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let mut db = Connection::open_with_flags(&self.database_path, flags)?;
        db.busy_timeout(std::time::Duration::from_secs(1))?;
        db.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA trusted_schema=OFF;")?;
        let version = scalar(&db, "PRAGMA user_version")?;
        if !(0..=2).contains(&version)
            || (version == 0
                && scalar(&db, "SELECT COUNT(*) FROM sqlite_master WHERE type='table'")? != 0)
        {
            return Err(StorageFailure::Incompatible.into());
        }
        if !write && version < 2 {
            drop(db);
            return self.open(true);
        }
        if write || version < 2 {
            self.check_capacity(0)?;
            let page_size = scalar(&db, "PRAGMA page_size")?;
            let max_pages = self
                .budget
                .database_capacity(&self.database_path, page_size)?
                / page_size;
            if max_pages < scalar(&db, "PRAGMA page_count")? || max_pages < 8 {
                return Err(StorageFailure::Limit.into());
            }
            db.execute_batch("PRAGMA journal_mode=DELETE;PRAGMA synchronous=FULL;PRAGMA auto_vacuum=INCREMENTAL;PRAGMA cache_spill=OFF;")?;
            db.execute_batch(&format!(
                "PRAGMA max_page_count={max_pages};PRAGMA journal_size_limit=0;"
            ))?;
            if version < 2 {
                let transaction = db.transaction()?;
                if version == 0 {
                    transaction.execute_batch("CREATE TABLE snapshots(id TEXT PRIMARY KEY,root TEXT NOT NULL,completed INTEGER NOT NULL,payload BLOB NOT NULL);")?;
                }
                transaction.execute_batch("CREATE TABLE cleanup_reports(id TEXT PRIMARY KEY,created INTEGER NOT NULL,payload BLOB NOT NULL);PRAGMA user_version=2;")?;
                transaction.commit()?;
            }
        }
        Ok(db)
    }
    fn failure(&mut self, message: String) -> HistoryWriteResult {
        self.last_user_message = Some(message.clone());
        HistoryWriteResult {
            saved: false,
            user_message: Some(message),
        }
    }
}
fn scalar(db: &Connection, sql: &str) -> Result<i64> {
    Ok(db.query_row(sql, [], |r| r.get(0))?)
}
pub(crate) fn lock_cancellable<'a>(
    gate: &'a Mutex<()>,
    token: &CancellationToken,
) -> Result<std::sync::MutexGuard<'a, ()>> {
    loop {
        token.check()?;
        match gate.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(poison)) => return Ok(poison.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::park_timeout(std::time::Duration::from_millis(5))
            }
        }
    }
}
fn make_room(db: &Connection, incoming: i64, capacity: i64, page_size: i64) -> Result<()> {
    let incoming_pages = (incoming + page_size - 1) / page_size * page_size + page_size;
    loop {
        let used:i64=db.query_row("SELECT COALESCE(SUM(((length(payload)+?1-1)/?1+1)*?1),0) FROM (SELECT payload FROM snapshots UNION ALL SELECT payload FROM cleanup_reports)",[page_size],|r|r.get(0))?;
        if used + incoming_pages <= capacity - SCHEMA_RESERVE {
            return Ok(());
        }
        if scalar(db, "SELECT COUNT(*) FROM cleanup_reports")? > 0 {
            db.execute("DELETE FROM cleanup_reports WHERE id=(SELECT id FROM cleanup_reports ORDER BY created,rowid LIMIT 1)",[])?;
        } else if scalar(db, "SELECT COUNT(*) FROM snapshots")? > 0 {
            db.execute("DELETE FROM snapshots WHERE id=(SELECT id FROM snapshots ORDER BY completed,rowid LIMIT 1)",[])?;
        } else {
            return Err(StorageFailure::Limit.into());
        }
    }
}
/// .NET DateTimeOffset.UtcTicks: 100 ns units since 0001-01-01, not Unix milliseconds.
pub fn dotnet_utc_ticks(time: chrono::DateTime<chrono::Utc>) -> Result<i64> {
    time.timestamp()
        .checked_mul(10_000_000)
        .and_then(|n| n.checked_add(i64::from(time.timestamp_subsec_nanos() / 100)))
        .and_then(|n| n.checked_add(621_355_968_000_000_000))
        .ok_or_else(|| anyhow::anyhow!("Timestamp out of supported range"))
}
fn storage_message(error: &anyhow::Error) -> String {
    if error
        .downcast_ref::<StorageFailure>()
        .is_some_and(|e| matches!(e, StorageFailure::Limit))
    {
        return "Local history storage budget is full. The current scan remains available.".into();
    }
    if error.downcast_ref::<StorageFailure>().is_some()
        || error.downcast_ref::<serde_json::Error>().is_some()
    {
        return "Local history is corrupt or incompatible. Reset history explicitly to resume saving.".into();
    }
    if let Some(rusqlite::Error::SqliteFailure(code, _)) = error.downcast_ref::<rusqlite::Error>() {
        if matches!(
            code.code,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
        ) {
            return "Local history is corrupt. Reset history explicitly to resume saving.".into();
        }
        if code.code == rusqlite::ErrorCode::DiskFull {
            return "There is insufficient space for local history. The current scan remains available.".into();
        }
    }
    "Local history storage is unavailable. The current scan remains available.".into()
}
