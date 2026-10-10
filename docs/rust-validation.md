# Rust client validation — 2026-10-11

## Alpha.6 map panel, hierarchical marks and compatible launch candidate

The map sidebar adds marked outermost roots, observed free space, asynchronous
conditional physical reclaim/free-after projection and informational recommendations.
Parent marks absorb descendants; covered children identify the parent and offer
unmarking that parent. Covered toggles preserve a valid manual review.

The forecast uses a separate unissued native observation, never normal preview
authority. It excludes unknown/foreign allocation and multi-link identities, and
checks scan UUID, selection generation, cancellation and root identity before
publication. Limits are 2000 roots, 100000 paths, 64 MiB conservative metadata
reservations, 1 MiB selected input, 16 KiB per path, depth64 and a shared cooperative
3-second deadline shared from root pinning through inventory and volume observation.
Cancellation/deadline checks precede each native call, including ancestor loops.
A single OS call cannot be forcibly preempted.
Partial/unknown metadata does not promise final free space. Confirmed deletion
waits in its background worker for canceled observation/Git leases to be released;
busy launch requests cannot interrupt a deletion attempt. TXT/JSON export workers
also await these leases before atomic non-overwriting publication; rejected busy
exports never take/join observers on the event thread. Root ancestor chains remain
pinned before and through both handle-based identity probes, rejecting replacement
junctions before reading their descendants.

Compatible map/manual-review shortcuts retain editor/modal and separate-confirmation
priority. Positional Unicode/relative folders, disk/display/worker flags and the
existing diagnostic switches resolve in the sender and open an ordinary scan/map.
Second normal launches forward only strict read-only overrides via local native
message pipes: same SID/executable peers, remote rejection, bounded 16-message queue,
64 KiB messages and a shared 3-second client deadline. Root validation caps paths at
64 components and 16 KiB before metadata; pinned ancestors are opened sequentially.
The helper remains a separate exact grammar; `--` positional data cannot activate it.

Largest-folder projection ranks directory references before resolving visibility,
and stops those path lookups after 2000 visible rows. Hidden higher-ranked rows do
not consume the visible limit; equal-sized rows retain their original order.
The missing-root native fixture uses an owned temporary directory on an available
local drive, rather than assuming a fixed drive letter exists on hosted runners.

Final full-suite, immutable review, portable EXE and hosted CI results are recorded
with the exact source revision in the candidate's verification artifacts. Programmatic
frames and actual EXE checks are distinct from manual UAC/tray/logon acceptance.

## Alpha.5 navigation/display candidate

Adds age/depth/hidden projections, pointer/keyboard and breadcrumb sibling navigation,
ordinary above-root widening with a bounded identity-checked subtree cache, adjustable
sidebar, system appearance, independent persisted interface scale, selected TXT and
advisory prompt rendering. Native deletion authority/helper/history protocol remain
unchanged. Existing published alpha.3 and stable 0.2.2 assets remain unchanged.

Focused runtime/frame verification passes: widening success/cancel/failure retention,
marks/full snapshot preservation, hidden lists/search/navigation, settings restart,
new-file export without overwrite or deletion, native editor typing and separate map/UI
zoom. Minimum 880×600 frames cover 18 RU/EN, light/dark/system, 75/100/150% variants.
A reproduced 150% viewport overflow was fixed with compact fixed header padding.

Cache regressions reproduced a root replacement binding race and canonical/raw path
mismatch (admitted but unused Known subtree). Before/after native identity plus exact
last-write precision and correctly spelled walker paths fix both; five cache cases
pass. The cache is bounded by 32 MiB/100000 entries/depth64 and cleared on deletion.
Cached observations are labelled rather than claimed fresh. Hidden navigation and
Largest-list leaks were reproduced and fixed while retaining the full scan and marks.

Independent review reproduced two additional defects with real frames: hovering
the retained map during widening could leave a numeric target bound to the old
index, and dragging a 420px sidebar capped by the minimum window had a dead zone.
Accepted scan generations now invalidate transient targets and old index actions;
sidebar dragging starts from its actual rendered width. Both regressions pass.
Six pages at 150% also retain their controls, headers and useful table/map space.

Locked Windows workspace: **385 passed, 0 failed, 7 ignored** (application111;
engine34; services33; native/platform43; vendored core164). Six ignored Git entrypoints
are exercised by parent tests; one is the existing upstream timing test.
Whole-workspace format and all-target strict Clippy pass. Portable source/hash and
hosted CI evidence is recorded for the final clean head after this pass. These are programmatic frames and real EXE checks, not
manual UAC approval/denial, tray/logon or desktop visual acceptance. macOS-specific
monitoring/permission and upstream Trash flows are outside this Windows port.

## Alpha.4 insights candidate

The alpha.4 development candidate adds indexed categories/reclaim hatching,
bounded recommendations, safe local Git inspection, conservative native reclaim
projection, measured volume changes and post-attempt refresh with report retention.
The published alpha.3 and stable C#/WPF 0.2.2 assets remain unchanged.

Locked Windows workspace verification: **331 passed, 0 failed, 7 ignored**
(application 71; engine 25; services 28; native/platform 43; vendored core 164).
Six ignored Git helper entrypoints are invoked and verified by their parent
tests; the remaining ignored test is the existing upstream timing test.
`cargo fmt --all --check` and workspace/all-target Clippy with `-D warnings` pass.

- Real generated Git repositories cover changed/untracked/stash/ahead,
  no upstream, missing Git, unsafe filters/includes, configuration changes and
  poisoned environment. Windows job tests prove cancellation/timeout kill the
  process tree, output is bounded and repository-selected external programs
  cannot start. Git reads only the selected checkout root and never fetches.
  An independent P1 review finding was reproduced with a real sibling marker
  `git.exe` nominated by poisoned PATH; OS-known-installation-only discovery
  makes the regression pass without executing it.
  Fixture ownership is checked against the canonical scratch parent and generated
  directory prefix, on any drive. Hosted CI exposed a local-only E: assertion;
  its regression now covers runner/local roots and rejects unrelated or nested paths.
- Runtime tests repeat Git checks after on-disk changes, reject cancelled/stale
  publications, delete only generated fixtures, refresh the accepted snapshot,
  and retain the actual report and SQLite journal when refresh fails.
- Engine/native tests cover inherited/contextual classification, nonoverlap,
  incomplete coverage, bounded recommendations and excluded hardlink/unknown/
  foreign identities. Every multi-link identity is excluded from physical
  projection because existing per-name link-count revalidation can prevent
  removal of a subsequent reviewed alias. Native deletion authority is unchanged.
- GPUI frame tests at 880×600 cover localized recommendations in light/dark,
  recommendation navigation without marking/deletion, six-page settings/map
  controls and the existing minimum 180 px map area with long paths.
  An independent P2 review finding was reproduced with eight long review paths.
  The actual modal now retains visible inventory and separate confirmation
  controls, with scrolling to the final warning in RU/EN and both appearances.

These are programmatic frames and generated native fixtures, not manual desktop
acceptance. Portable archive/extracted-EXE evidence is recorded separately with
its exact source commit, SHA-256 and verification scope after building. UAC
approval/denial, tray clicks, logon/autostart and manual visual acceptance remain
pending. Navigation/age/depth, system theme/UI scaling and selected-path text/
prompt export are remaining port phases.

## Previous alpha.3 baseline

Current version: **0.3.0-alpha.3**, Windows x64. The user authorized merging Rust as the
primary client in `main`, accepting the disclosed preview limitations. That
approval changes the development baseline; it is not evidence of manual desktop
acceptance. C#/WPF source is preserved at tag `v0.2.2`; published stable
**0.2.2 is unchanged**. Rust alpha.3 remains an unsigned prerelease.

The completed evidence below belongs to the previous validated alpha.2 source `90d8091`. Documentation,
CI and source cleanup for the main-branch transition do not retroactively prove
an executable built from a later commit. New package provenance must identify
its own source revision and validation run.

## Alpha.3 source validation

The current Rust-only source passed `cargo fmt --all --check`, strict Clippy
for all workspace targets and the full locked Windows suite: **281 passed,
0 failed, 1 ignored** (application 43, engine 12, services 22 plus 6 retry
regressions, native cleanup 24, platform policy 10, vendored scanner 164).
The new scheduler regression reproduced an immediate second request before the
fix. Runtime regressions cover failure, cancellation, manual system-volume
attempts, immediate manual scans of another root and stale worker tokens.
Independent source review and a separate rereview found no remaining blockers
in the changed scheduling paths; a possible test-clock flake was corrected.
Package/hosted validation is tracked separately from these local checks.

## Earlier alpha.2 passing local checks

`cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked
-- -D warnings`; `cargo test --workspace --locked`.
**269 passed, 0 failed, 1 ignored**: application 37, engine 12, compatible
services 22, native cleanup 24, platform policy 10, vendored scanner 164.
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
  and hiding of a window belonging to this process only. The GPUI Windows
  platform's global activate/hide methods are no-ops; the app uses its own
  checked native window handle for these operations.
- Actual ordinary-user EXE: read-only disposable fixture scan, live map
  geometry, four localized appearance projections and compatible JSON export.
  Fresh native startup produced a nonzero window handle and the expected title
  within one second. This verifies window existence, not visual correctness.
- Release uses static CRT. PE imports contain Windows OS DLLs; no .NET,
  external SQLite, VCRUNTIME or MSVCP runtime. Original blue icon and preview
  version are embedded; GPUI supplies its ordinary-user/DPI manifest.

## Ordinary scan budget regression

Alpha.1 charged four copies of the full path for every enumerated file, although
ordinary leaves retain basenames in the tree/index and only the largest 100 files
receive history paths. It also charged entries excluded by scan filters.
Alpha.2 reserves full paths for directories and diagnostic entries, includes the
root path once, and counts only retained entries. The 1 GiB estimated resident / 20 million retained-entry
ceilings and rejection of incomplete results on genuine exhaustion are unchanged.
Failures now include numeric entry and estimated-memory usage without file paths.

Regression checks cover 128 files under a long root, filtered entries, long
directory paths that genuinely exhaust the resident budget, and both hard limits.
A read-only ordinary scan of the real C: drive reproduced the original error at
approximately 175 MB peak working set. With the fix it completed 1,235,987 files
and 264,460 directories in approximately 12.4 seconds at 282 MB peak working set.
These measurements cover the scanner process, not the complete GUI/history.
Protected/unreadable areas remain explicitly incomplete; traversal completion
does not assert access to every object on the volume. The resident budget is an
estimated metadata reservation, not a hard cap on process working set.

## Extracted alpha.2 application on C:

The portable archive from source `90d8091` was freshly extracted and its actual
ordinary-user executable completed a read-only C: scan in approximately
**41 seconds**, with peak working set **627,806,208 bytes** (about 628 MB).
Its proof contains **264,508 directory observations**, **100 largest files**,
**830 issues**, **412 map tiles** and four RU/EN light/dark model projections.
A fresh SQLite database actually retained one completed snapshot (about
**53.4 MB**); the JSON snapshot export was about **64.4 MB**. The native window
also started with the expected title and a nonzero handle.

These are application, map-model and persistence checks. They do not establish
manual visual inspection, complete access to protected directories or a real
UAC approval flow. Scanner-only measurements above belong to a separate run on
a changing volume and should not be substituted for whole-application memory.
History permits at most 30 snapshots, but its 250 MiB storage budget can prune
large inventories sooner; no fixed promise of 30 retained whole-volume scans is
made. The map's live index and exported snapshot have different file retention.

## Follow-up source review

Independent review of `90d8091` found that failed/cancelled automatic scans could
retry immediately. The follow-up correction defers failed
background scans for at least 15 minutes using monotonic time and cancelled
ones until the configured interval, while allowing an immediate manual scan.
Automatic requests reserve a generation until their current worker completes;
failed system-volume manual scans also wait 15 minutes before any automatic
retry, and manual cancellation postpones automatic work for the selected interval.
Other roots and non-scan operations do not affect the automatic reservation.
The 269-test and extracted alpha.2 evidence above predates this correction.

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
- Fresh package/hosted validation of source changes made after `90d8091`. An independent
  source review of `90d8091` has completed: no P0/P1 findings were reported.
  It found a P2 automatic-scan retry issue after failure/cancellation, corrected
  with regression tests and rereviewed. This review is not a fresh acceptance test
  or proof that later edits have been reviewed.

The alpha.3 ZIP is an unsigned preview; its final executable/package verification must be recorded before publication. The user has authorized the source replacement
and merge into `main` despite the pending manual checks; the project must keep
those limitations visible. Stable 0.2.2 is retained unchanged. Publishing Rust
alpha.3 requires a separately validated archive and records it as a prerelease,
not as evidence that the outstanding desktop checks passed.
CI artifacts are additional reproducible checks, not manual desktop acceptance.
No passing hosted CI status or completed GitHub publication is asserted here.
Hosted Windows runners run elevated with UAC disabled
([GitHub runner documentation](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)).
The CI-only verifier therefore creates a temporary standard account on its
disposable VM and checks `ordinary_user` in the extracted executable's proof.
Its local guard rejects execution outside GitHub Actions. The product's refusal
to run its main process elevated remains unchanged.
