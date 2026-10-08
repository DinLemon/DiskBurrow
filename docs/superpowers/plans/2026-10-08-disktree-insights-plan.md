# DiskBurrow Insights Implementation Plan

> **For agentic workers:** use superpowers:executing-plans task by task, with independent parallel workers for tasks 1–3 and one final whole-change review. The user previously selected subagents; preserve that execution preference. Root owns integration and commits.

**Goal:** complete the first two priority groups of the remaining disktree port as working, tested application behavior.
**Architecture:** extend the existing live-index analytics, background operation flow and native reviewed inventory; no new client or elevated mutation process.
**Tech Stack:** Rust 1.97, Windows MSVC, GPUI, existing native APIs and vendored disktree-core.
**Spec:** ../specs/2026-10-08-disktree-insights-design.md

## Global Constraints

- Windows x64; all build/cache/generated fixtures on owned E:.
- Permanent, separately confirmed deletion only; protected/cloud/reparse/identity rules remain authoritative.
- Git is read-only/local/bounded/cancellable; unsafe or unavailable information is unknown, never clean.
- Physical-byte projection is conservative and avoids duplicate hardlink/allocation credit.
- No changes to compatible persisted settings/history semantics or helper authentication.
- RU/EN and light/dark; no claimed manual UAC/tray/logon/layout acceptance.

## Review Focus

- A configured Git filter or environment must not execute a repository-chosen program.
- External hardlinks, unknown allocation and another volume must not inflate reclaim claims.
- Parent and child recommendations must not offer duplicate byte totals.
- Cancelled or stale background work must not publish a clean Git result for a different selection.
- Post-deletion scan errors must preserve the real deletion outcome and journal.

### Task 1: Indexed classification and insights

**Files:** engine `src/analytics.rs` (new), `src/lib.rs`, `src/live_index.rs` if needed; engine tests only. Root owns app integration.
**Interfaces:** consumes `LiveIndex`; produces category/reclaim classification and `recommendations(now: i64, limit: usize)`. Return records identify the live entry index and reason, known allocated bytes, logical bytes and coverage. No filesystem probes; no persisted DTO/helper protocol changes.
- [x] Write generated-index tests for inherited cache/build categories, contextual siblings, parent-child suppression and unknown coverage.
- [x] Run engine tests to observe RED behavior before implementation.
- [x] Implement bounded analytics and document the exact public API in the worker report.
- [x] Run engine tests, formatting and strict crate lint; record commands/results.

### Task 2: Safe Git inspection

**Files:** app `src/git_inspection.rs` (new), app module declaration in `main.rs`; app Cargo features only if needed for a Windows process job. Root owns runtime/UI.
**Interfaces:** `inspect(path: &str, cancel: &diskburrow_windows::Cancellation) -> GitInspection`; returned fields identify repository/not-repository/unknown plus tracked/untracked/stash/ahead and the reason for uncertainty. Inspection timeout 5 seconds total, bounded stdout/stderr 1 MiB, no network or interactive prompt. Nested subprocess lifetime belongs to a kill-on-close Windows job; cancellation closes it.
- [x] Test real generated repositories and absence of an upstream; unsafe filter/config must not run a marker command.
- [x] Observe RED; implement safe discovery/command environment, timeout/output limits and cancellation.
- [x] Test cancellation and bounded child lifetime; report the exact API and native job evidence.
- [x] Run focused tests and strict crate lint without modifying runtime/UI.

### Task 3: Conservative manual reclaim and actual volume change

**Files:** windows `src/reclaim.rs` (new), `src/cleanup.rs`, `src/lib.rs`, native tests. Root owns runtime/UI.
**Interfaces:** a projection over `ManualDeletePlan` for a requested volume, with known reclaim bytes and excluded hardlink/unknown/foreign counts; before/after measurements populate the existing `CleanupReport` free-space delta fields.
- [x] Write RED tests for unique allocation, duplicated identities, complete versus external hardlink sets and unknown/foreign entries.
- [x] Implement the pure estimate and capture same-volume free-space before/after execution without changing deletion admission/authority.
- [x] Cover unavailable and negative volume changes and partial/cancelled outcomes; preserve existing native tests.
- [x] Run Windows focused tests and strict crate lint; report exact API and scope.

### Task 4: Integrate application behavior

**Files:** app `contract.rs`, `runtime.rs`, `ui.rs`, `map_view.rs`, `locale.rs`, `recommendations.rs`, tests.
- [x] Add failing runtime/frame tests for recommendation selection, RU/EN category/Git/reclaim fields and preserved result after refresh.
- [x] Publish analytics with each accepted scan, avoid full-index work during rendering; add colors/legend/reclaim hatching and bounded recommendations.
- [x] Inspect selected repositories in the background and repeat during manual preview; show current-generation inspection disclosure in the review before permanent confirmation.
- [x] Show projection excluding uncertain allocation, and measured result with concurrent-volume-change disclosure.
- [x] Automatically rescan the last deletion root after worker completion; retain report/journal and explicit refresh failures/cancellation.
- [x] Run focused application tests, then the complete locked workspace suite, formatting and strict lint.

### Task 5: Review and portable evidence

- [x] Independent source review of the immutable diff, native process/deletion boundaries and transient publication states.
- [x] Correct any load-bearing findings and rerun their covering checks.
- [x] Update validation and user docs to the implemented scope; retain all unported items and manual acceptance checks.
- [ ] Build a fresh portable archive, verify extracted ordinary-user fixture scan/map/export, checksums and source revision.
- [ ] Publish a reviewable GitHub PR with exact validation scope; attach it to this task. Do not modify existing releases or tag incomplete behavior as stable.
