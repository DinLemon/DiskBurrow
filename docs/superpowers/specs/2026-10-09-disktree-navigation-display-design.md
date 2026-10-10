# DiskBurrow: remaining Windows navigation, display and handoff

Continuation of the approved full Windows disktree port. Baseline is the verified
alpha.4 head `0dc886a`; PR #5 remains reviewable. Preserve permanent deletion with
separate confirmation and all native no-follow/cloud/identity/protected rules.

## Outcome

- Age colors use last-write metadata: unknown/future, <=7, <=30, <=180, <=365,
  and >365 days. Age changes color, not area; the chosen physical/logical/files
  metric continues to size tiles. RU/EN legend explicitly says age is not junk.
- Draw 1..6 levels (default 3). Hidden display excludes dot-prefixed and Windows
  hidden/system subtrees from map/list/search projection, without deleting data
  or changing the retained full scan. Show the full and visible totals distinctly.
- Breadcrumb sibling picker ranks by selected metric. Above-root breadcrumbs
  widen an ordinary local scan; retain the previous view on failure/cancellation
  and focus the old root on success. Reuse only a bounded, compatible ordinary
  subtree cache with unchanged native root identity; otherwise do a fresh walk.
  Reused observations are identified as cached, never claimed fresh.
- Wheel zoom enters a directory once it fills the canvas; zooming out at base
  scale goes up. Pointer movement versus keyboard movement selects the active
  target. Tab cycles siblings; right click reveals the target in Explorer.
  Add upstream shortcuts, localized help, search escape/isolation, and rescan.
- Selection details show share, last write, files, category and current Git info.
  Sidebar is adjustable/resettable and remains usable at 880x600.
- System theme follows AppsUseLightTheme (bounded read-only Windows registry
  polling). Manual light/dark override it. Interface zoom is independent of map
  zoom: 75/90/100/110/125/150 percent, persisted with explicit reset. Existing
  settings deserialize with defaults; no history/helper protocol changes.
- Save selected outermost indexed paths to a new UTF-8 TXT file and copy a
  localized agent review prompt. Escape controls/markup; paths remain data.
  Copy/export never executes deletion or sends a message. Keep snapshot JSON.

## Limits and evidence

Windows x64 only; do not add Recycle Bin, link following, cross-volume policy,
or a general CLI. Local build/cache/test writes use owned E: scratch. Destructive
fixtures are generated there; C: validation is read-only with E: data storage.
Retain all current cancellation/generation gates and bounds. Subtree cache is
at most 32 MiB / 100000 entries / depth 64; unknown/changed roots and fast-MFT
snapshots use a fresh traversal. Cache is cleared after deletion attempts.
Programmatic GPUI frames/native/EXE checks do not claim manual UAC/tray/logon
acceptance. Finish with independent review, full locked suite, strict lint,
portable extracted-EXE proof and hosted CI. Publish updated PR; no stable tag.
