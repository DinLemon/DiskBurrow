# DiskBurrow: map panel, hierarchical marks and compatible launch

The user approved these three remaining Windows groups after the verified alpha.5
candidate at 80fac8c. Continue on feature/disktree-insights and update PR5, without
merging or publishing a release. Keep the existing permanent-delete policy and
separate confirmation, no-follow/cloud/protected/native identity checks, helper
protocol and history compatibility. Windows x64 Rust/GPUI; owned E: scratch only.

## User-visible behavior

- The map sidebar shows bounded selected outermost roots (unmark individually or
  clear all), observed volume free space, an asynchronous conditional reclaim/free
  forecast and the existing indexed Worth-a-look recommendations. Recommendation
  clicks navigate only. Selected roots remain visible across hidden projection.
  Lists/panel scroll at 880x600 and 75/100/150%; no fixed-height stack can starve
  the canvas. Totals, truncation, stale observations and unknown values are explicit.
- Marking a parent absorbs existing descendant marks. A child of a marked parent
  cannot become an independent mark or exclusion; show the covering parent and an
  action to unmark that parent. Only actual selection changes invalidate a deletion
  review. Component/case-aware Windows boundaries preserve neighbouring folders.
- Forecast metadata is collected by a bounded cancellable read-only native worker,
  using the current outermost roots and volume identity. Exclude unknown/foreign
  allocation and multi-link identities. Do not synthesize reclaim from logical size.
  This worker never creates a review, stores deletion authorization or performs Git,
  deletion, content reads or hydration. Publication checks scan UUID, mark generation
  and cancellation. Scan/root/selection/deletion/exit changes invalidate it. A fresh
  normal preview remains mandatory before separately confirmed deletion.
- Match upstream supported Windows shortcuts: space/x mark; arrows and hjkl move;
  Tab siblings; Enter descend; backspace/u ascend; Escape clears search, cancels scan,
  dismisses selection then ascends; alt arrows and mouse side buttons navigate;
  t cycles Size/Files/Age, d allocated/apparent, i hidden, r/F5 rescan, g whole system
  disk, v volumes, p selection details, o Explorer, c review, / or s search, ? help,
  q exit, CtrlO folder picker, Ctrl+/-/0 interface scale. Editors retain text keys.
  Review shortcuts s save TXT, a prompt, ! clear, Escape dismiss, Enter opens the
  existing separate permanent confirmation only. p keeps permanent mode; m cannot
  select an unsupported Trash mode. Confirmation gates and modal focus take priority.
- Accept a positional local directory (including spaces/Unicode and relative paths
  resolved by the launching process), --disk/-D, --apparent-size/-a, --no-hidden/-H,
  --depth/-d 1..6, --metric bytes/size/files, --one-filesystem/-x, -h/--help and --
  positional separator. Preserve existing --data-dir/--scan-root/--background/
  --verify-runtime flags and strict helper argument parsing. Conflicting roots,
  missing/invalid values and unsupported --follow-links/-l or --cross-filesystems/-X
  fail explicitly under the already-chosen single-volume no-follow policy.
- Compatible bounded scan-worker CLI flags use existing vendored ScanThreads:
  --power-efficiency miser/balanced/aggressive/drain-my-battery, --scan-threads,
  --adaptive-threads/--fixed-threads, --thread-throughput-percent 1..100 and
  --thread-system-cpu-percent 0..100. Session overrides do not silently persist;
  fast NTFS uses its existing separate helper policy, not ordinary worker controls.
- An explicit launch directory/options starts an ordinary scan and opens the map.
  A second normal launch forwards only validated read-only launch overrides to the
  existing same-user instance. Bounded native IPC pins peer identity and cannot
  carry helper credentials, paths to settings/data, exports, deletion or commands.
  Resolve sender-relative paths before forwarding; busy work must finish/cancel
  safely before a queued root is accepted, never interrupt a deletion attempt.

## Validation and deliverable

TDD for each pure module plus real owned Runtime/native IPC/GPUI regressions.
Verify parent/child/sibling case boundaries, unknown/hardlinked/foreign estimates,
stale worker publications, review/typing shortcut isolation and minimum layout.
Full locked suite, fmt, all-target strict Clippy, independent immutable source
review, a fresh portable alpha.6 with extracted ordinary-user EXE checks and hosted
CI. Manual UAC/tray/logon/desktop acceptance stays explicitly pending. Only the
final deliverable and sanitized proofs go to C outputs; no private C inventories.
