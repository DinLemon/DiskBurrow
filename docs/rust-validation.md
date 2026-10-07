# Rust preview validation — 2026-10-07

Candidate: **0.3.0-alpha.1**, branch `rewrite/rust-gpui`, Windows x64.
Published C#/WPF **0.2.2 is unchanged**. This is a migration candidate, not a
claim that all desktop acceptance checks have been completed.

## Passing local checks

`cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked
-- -D warnings`; `cargo test --workspace --locked`.
**266 passed, 0 failed, 1 ignored**: application 37, engine 12, compatible
services 22, native cleanup 24, platform policy 10, vendored scanner 161.
The ignored upstream timing test is retained as ignored, not counted as passing.

- GPUI frame/control tests: six pages, RU/EN, light/dark, editable decimal GB,
  Save visible at 880×600, long root/breadcrumb map with at least 180 px canvas,
  shared review dismissal. These use GPUI's test platform.
- Runtime: ordinary scan, full live index, logical/allocated map metrics,
  hard-link accounting, exact reasons for incomplete coverage, cancellation,
  snapshot export without replacing existing files, retained history after
  restart, independent volume counters and coverage, observed-cache hints.
- Compatible persistence: PascalCase C# payloads, .NET UTC ticks and omitted
  legacy thresholds, schema migration, corrupt-data reporting without reset,
  concurrent storage budget and rollback of failed writes.
- Windows native fixtures: handle-bound observations/deletion, immutable
  single-use previews, identity and parent replacement, new/busy children,
  reparse/cloud/protected boundaries, cancellation and partial outcomes.
  Only freshly generated fixture trees are deletion targets.
- Integration policies: private same-user named-pipe multi-chunk transfer,
  cancellation while native I/O is pending, bounded protocol/error frames,
  autostart compensation that preserves concurrent changes, and restoration
  of a hidden window belonging to this process only.
- Actual ordinary-user EXE: read-only disposable fixture scan, live map
  geometry, four localized appearance projections and compatible JSON export.
  Fresh native startup produced a nonzero window handle and the expected title
  within one second. This verifies window existence, not visual correctness.
- Release uses static CRT. PE imports contain Windows OS DLLs; no .NET,
  external SQLite, VCRUNTIME or MSVCP runtime. Original blue icon and preview
  version are embedded; GPUI supplies its ordinary-user/DPI manifest.

## Outstanding acceptance checks

Supported desktop helper initialization failed twice in this validation run:
`failed to write kernel assets: Системе не удается найти указанный путь.
(os error 3)`. Native desktop automation is otherwise unavailable here.
No substitute desktop driver or custom Win32 UI automation was used.

- Manual navigation and visual inspection of every page/column at minimum and
  normal size, both languages and appearances; actual scan/review/cancel clicks.
- Real Rust UAC approval and denial on the secure desktop, followed by a real
  whole-volume NTFS scan and cancellation. The previous C# UAC test is not Rust
  evidence. Protocol/native-pipe tests do not cover the system prompt.
- Actual tray menu clicks, hide/second-launch restoration and Exit while busy;
  autostart after Windows logon. Unit policy checks are narrower evidence.
- Independent whole-branch review. Component workers stopped at their account
  usage limit; no successful independent final review is claimed.

The ZIP is an unsigned preview for review/testing. No stable merge, replacement
release or automatic publication is performed before these checks are resolved.
CI artifacts are additional reproducible checks, not manual desktop acceptance.
