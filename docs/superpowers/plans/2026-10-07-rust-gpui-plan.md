# DiskBurrow Rust + GPUI Implementation Plan

> For agentic workers: execute in this session with independent native/service workers and a final whole-branch review. TDD cycle for each new behavioral component; existing vendored upstream tests are retained.

**Goal:** replace all 0.2.2 functionality with Rust+GPUI without changing stored user data semantics or safety guarantees.
**Architecture:** vendor scanner/layout; canonical compatible service DTOs; native handle-bound cleanup; UI-thread GPUI state with background workers and bounded events.
**Tech Stack:** Rust1.97/windows-msvc, gpui-kit0.6.6, gpui-omarchy0.1.3, windows-sys0.61, rusqlite bundled, serde/chrono/uuid.
**Spec:** ../specs/2026-10-07-rust-gpui-design.md

## Global constraints
Windows x64; all build/dependency/scratch writes on owned E; same-user UAC reader only; no deletion without reviewed confirmation; existing 0.2.2 release remains immutable. Child tasks own separate crates; root owns workspace, scanner/runtime/UI and integration. Product version0.3.0-alpha.1 until complete migration validation.

## Review focus
- Reparse/protected boundary replacement between preview and deletion: identity-bound handles, fail closed.
- Legacy omitted JSON defaults/date precision/schema payloads: fixture roundtrip exact, no actual AppData tests.
- Large live index/partial scan: bounded UI projection, cancellation, incomplete coverage retained, no truncated live tree passed off as full.
- UAC over-the-shoulder credentials/spoofed or oversized IPC: same SID and authentication, limits and final parent disappearance/cancel.
- Both locale/palette/minimum sizes during asynchronous publications: UI-thread state, actual rendered/interactive verification.

### 1. Workspace and upstream baseline (root + rust_build)
- [x] Vendored core/source license and locked dependency setup; branch baseline/build environment inspected before adoption.
- [x] Verify actual Windows GPUI build with task-local Rust/MSVC; logs and executable provenance on E.
### 2. Compatible settings/history/budget/scheduler (rust_storage)
Files rust/crates/diskburrow-services/src/{models,settings,history,budget,monitor}.rs.
Produces AppSettings, SettingsStore, StorageBudget, SqliteHistoryStore, ScanSnapshot/FileObservation/ScanIssue/CleanupReport canonical DTOs, MonitoringScheduler tick actions.
- [x] Write/run failing exact C# JSON/SQLite ticks, threshold migration, caps and scheduler/battery/coalescing fixtures.
- [x] Implement persisted behavior; run crate tests, fmt/clippy; report every failure and API to root.
### 3. Native reviewed cleanup (rust_native_cleanup)
Files rust/crates/diskburrow-windows/src/{observation,rules,manual,paths}.rs.
Consumes canonical DTOs. Produces CleanupService native inspect, preview_cleanup/execute_cleanup, preview_manual/execute_manual with single-use UUID plans.
- [x] RED native own-tree identities/neighbors/replacement/protected/reparse/cloud/age/browser-state tests.
- [x] GREEN native implementation; preview mismatch/unconfirmed operations never mutate files; limits/cancel outcomes explicit.
### 4. Paired-size scanner and full live map (root)
Files vendor/disktree-core/src scanning/tree patches; app/src/{scanner,map_model}.rs.
- [x] RED native fixture logical/allocation/hardlink and incomplete coverage tests; add Node paired metadata without extra per-file probes in MFT.
- [x] Convert full tree to canonical bounded history snapshot; retain complete live tree in app; map all three metrics/filter/projection/selection/navigation tests.
### 5. Elevated helper and platform lifecycle (root, build worker after compile)
Files app/src/{platform,helper,runtime}.rs.
- [x] RED protocol root/SID/token/size/cancel tests and isolated registry adapter tests; implement ShellExecute runas/pipe reader, second instance and tray lifecycle.
- [x] All actions stay ordinary-user except read-only helper; runtime drives pure scheduler and journals completed work.
### 6. Six GPUI pages/localization/appearance (root + build worker after baseline)
Files app/src/{app,ui,locale,map_view}.rs.
- [x] Port RU/EN resource strings and byte formatting tests first; write actual GPUI frame/control tests for pages and live themes/languages.
- [x] Render bounded virtual rows/full index map and always-visible Save, background events arrive only through UI context.
### 7. Integration, real Windows GUI and portable package (root)
- [x] cargo fmt --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test --workspace; required upstream tests plus Rust parity fixtures green.
- [x] Build native release and isolated EXE scan/map/export proof; actual native window startup observed.
- [ ] Supported manual desktop page/theme/language/scan/review/cancel checks, real Rust UAC approval/denial, tray clicks and logon autostart. Sky initialization failed with Windows error 3; GPUI frame tests are programmatic evidence only.
- [ ] Publish allowlisted ZIP/checksum in rewrite branch/draft PR; attach PR. Merge/release only after complete parity, final fresh archive proof and whole-branch review.

Track results in work/rust-parity.md and user-facing outputs/DiskBurrow-rust-migration.md; preserve exact command/provenance and unresolved limitations.
