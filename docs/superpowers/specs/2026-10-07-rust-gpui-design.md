# DiskBurrow Rust + GPUI migration

Approved scope: full native Rust replacement of DiskBurrow 0.2.2 on the separate rewrite/rust-gpui branch. The published C#/WPF main and immutable releases remain available. Rust+GPUI, reuse of disktree algorithms, preservation of settings/history/tray/UAC/deletion rules, and independent component implementation are the selected approach.

## Architecture

Windows x64, Rust 1.97, GPUI via upstream-pinned gpui-kit 0.6.6/gpui-omarchy 0.1.3. Workspace rust/ with vendored MIT disktree-core at 6c8d4ce6211bf135ff4e42890faebff1040e1846 for NTFS reading and map algorithms; retain source license/notice. diskburrow-services supplies compatible PascalCase JSON/SQLite DTOs, settings, bounded history and pure scheduler policies. diskburrow-windows supplies native observation and identity-safe reviewed deletion. diskburrow-app owns ordinary scan adapter, elevated read-only helper, single-instance/tray/autostart runtime and six GPUI pages. No managed runtime or C# subprocess is used in the Rust product. Original C# source remains preserved at tag v0.2.2; locale compatibility fixtures are retained under the Rust app tests.

## Required behavior

- Six pages: Overview, Largest folders/files, Map, History, Cleanup, Settings; RU/EN live localization of controls, headers, row categories/statuses; live light/dark theme persisted only on Save; decimal GB, exact existing byte thresholds, new15e9/5e9 defaults. Min880x600 and1280x800 layouts, bounded paths/tooltips, pinned Save, finite map viewport >=180px.
- Live full tree: logical size separately from known allocated bytes, hardlinked physical sizes deduplicated, incomplete coverage/unknown allocation explicit; links/cloud entries skipped without hydration. Largest2000 dirs/100 files; cleanup2000 displayed rows with complete-plan filtering/selection semantics. Map preserves full index, three metrics, search/highlight/isolation/global/local scopes, breadcrumbs/back/forward/up, pointer zoom/pan/keyboard, review shared with Largest, exact parent-child selection coalescing.
- Local existing data %LOCALAPPDATA%/DiskBurrow. Settings PascalCase JSON, .NET TimeSpan strings and numeric enums compatible; omitted legacy byte thresholds retain binary defaults, newly missing document decimal defaults. SQLite user_version2 existing tables/payload UTC ticks remain readable; 30 completed snapshots/100 files retained, 250MiB combined budget/10MiB logs, cleanup journal, corruption reported without deleting user data, failure preserves live result. JSON export bounded snapshot with explicit disclosure; no telemetry/network.
- Monitoring: system root identified by Windows, selectable local drives/folders; default6h full scan, startup5min/free-space5min;1/6/12/24h choices; battery wait bydefault, nonoverlap/coalesced schedules; Pause stops full scans while free checks continue; daily growth alert per folder; cancellation/drain and safe process shutdown.
- Fast whole-volume NTFS: ordinary process launches only explicit read-only helper via UAC, same-account authenticated bounded IPC, checked volume/local NTFS, cancellation/secure prompt denial handled, malformed protocol fails closed. Cleanup never inherits elevation. Named streams/volatile MFT coverage limitations retained.
- Reviewed permanent deletion: separate preview and explicit confirmation; context/selection changes invalidate plan, native identities/metadata checked immediately before handle deletion. Protected system/profile/application containers refused; root complete inventory required; links/cloud/inaccessible/new/changed/busy entries preserved; partial/cancelled outcome journaled. Four cleanup rules exactly userTEMP>7d, CrashDumps .dmp>7d, Chrome/Edge HTTP cache only with closed/known browser state; safe approved custom TEMP and exclusions.
- Tray close hides; second launch restores existing user's window; tray Pause/Open/Exit; autostart off unless explicit saved choice, existing path checks; no automatic updates/security-setting changes.

## Acceptance and evidence

Promotion decision (2026-10-07): the user explicitly authorized replacing the
C# client in the repository's default `main` branch and continuing Rust
development. This supersedes the original pre-merge manual-parity gate below;
it does not claim the outstanding desktop/UAC/tray/logon checks have passed.
The release remains an unsigned prerelease until those checks are complete,
and C# source/release `v0.2.2` remains preserved and unchanged.

Native fixtures only for destructive tests, isolated owned E data paths for migration/budget tests. Rust fmt/clippy/test, real paired scanner sizes and links/cloud guards, SQLite/C# payload fixtures, bounded authenticated helper tests, no injected UI work from worker threads. Actual GPUI application built/started on Windows; supported Sky examines all pages with own fixture, both languages/themes, field/column layouts and cancel-only review. Final ZIP integrity/allowlist/runtime dependencies and independent extracted EXE tested before publication. Full parity matrix must be green or explicitly outstanding; no scaffold/partial migration presented as complete, no Rust replacement merged/released before parity. Caches/toolchains/target/logs/temp on E only; user outputs contain final artifacts/evidence.
