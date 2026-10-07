# DiskBurrow Rust + GPUI Implementation Plan

> For agentic workers: execute in this session with independent native/service workers and a final whole-branch review. TDD cycle for each new behavioral component; existing vendored upstream tests are retained.

**Goal:** make Rust+GPUI the primary client without changing compatible stored user data semantics or deletion safety guarantees.
**Main-branch decision (2026-10-07):** the user explicitly approved merging Rust to replace the active C# client, with the disclosed preview limitations. The target branch is `main` (the repository does not use `master`). Preserve immutable C# source/release at `v0.2.2`; archive its historical validation separately. Approval is not evidence of manual UAC, tray, logon or visual acceptance.
**Architecture:** vendor scanner/layout; canonical compatible service DTOs; native handle-bound cleanup; UI-thread GPUI state with background workers and bounded events.
**Tech Stack:** Rust1.97/windows-msvc, gpui-kit0.6.6, gpui-omarchy0.1.3, windows-sys0.61, rusqlite bundled, serde/chrono/uuid.
**Spec:** ../specs/2026-10-07-rust-gpui-design.md

## Global constraints
Windows x64; all build/dependency/scratch writes on owned E; same-user UAC reader only; no deletion without reviewed confirmation; existing 0.2.2 release remains immutable. Child tasks own separate crates; root owns workspace, scanner/runtime/UI and integration. Product version 0.3.0-alpha.3 remains a preview; source promotion does not imply completed manual validation.

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
- [x] Publish a local alpha.2 allowlisted ZIP/checksum, attach the draft PR, and run the freshly extracted ordinary-user EXE on C: with map/history/export proof (source `90d8091`).
- [x] Fix ordinary scan metadata accounting without increasing its 1 GiB estimated resident / 20 million retained-entry ceilings; genuine exhaustion still rejects a completed result. Local suite: 269 passed, 0 failed, 1 upstream timing test ignored.
- [ ] Promote Rust build/docs/assets/CI to the `main` baseline and remove active C# source/build entry points; preserve C# at immutable tag `v0.2.2` and its release.
- [ ] Obtain fresh passing package/standard-user CI evidence for the final source revision, publish alpha.3 as an unsigned prerelease if validated, and merge the authorized replacement PR. Keep all pending desktop checks disclosed.
- [x] Independent source review of `90d8091`: no P0/P1 findings reported; one P2 immediate automatic retry issue identified.
- [x] Correct the automatic retry finding with TDD (failed scans: at least 15-minute monotonic delay; cancelled scans: configured interval; manual scan remains immediate) and validate the final revision. Final local suite: 281 passed, 0 failed, 1 ignored; a separate rereview found no remaining scheduler blocker. The old alpha.2 executable proof does not cover alpha.3.

Track current results in docs/rust-validation.md and local runtime proof artifacts; preserve exact source/package provenance and unresolved limitations. Historical C# verification lives under docs/legacy-csharp/.
