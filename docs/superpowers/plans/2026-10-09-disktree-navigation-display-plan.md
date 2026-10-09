# Remaining disktree Windows UI Implementation Plan

> **For agentic workers:** use executing-plans with isolated parallel modules;
> root integrates and a fresh reviewer checks the whole change.

**Goal:** complete the remaining Windows navigation/display/selected-export flows.
**Architecture:** pure bounded engine projection; compatible settings and a
read-only appearance provider; pure selected export renderer. Root owns transient
contracts, runtime workers, GPUI interaction and scan-cache admission.
**Tech Stack:** Rust 1.97, GPUI-kit 0.6.6, existing vendored core and Windows APIs.
**Spec:** docs/superpowers/specs/2026-10-09-disktree-navigation-display-design.md

## Global constraints

Permanent separate confirmation; Windows x64/local/no-follow/cloud protections;
RU/EN/light/dark/system; owned E: fixtures; history/helper schemas unchanged.
Age bands 7/30/180/365 days; levels 1..6 default3. UI scale
75/90/100/110/125/150 percent. Sidebar 180..420 px default225.
Cache <=32MiB,100000 entries,depth64; no fast/changed-root/deletion reuse.

## Review focus

1. Hidden ancestors/search isolation must not leak descendants or alter marks.
2. Cached widening must preserve boundaries, identity, cancellation and budgets.
3. Text fields/dialogs must retain typing/confirmation under new shortcuts/zoom.
4. Missing registry values/settings additions must preserve previous user choices.
5. Malicious names/overlapping selections must not become executable instructions.

### Task 1: pure map projection (engine worker)

Files: engine live_index.rs, new display.rs and focused engine tests.
Interfaces: `MapProjection { depth:u32, show_hidden:bool }`;
`LiveIndex::layout_with_projection(root,bounds,metric,query,isolate,global,projection)`;
`LiveIndex::visible(index,show_hidden)->bool`; `AgeBand::from_modified(modified,now)`.
Existing layout stays compatible (3 levels, show hidden). No new filesystem I/O.
- [x] Write and observe RED for deep tiles, hidden ancestry/search and age bounds.
- [x] Implement bounded projection and age bands; cover invalid depth/time.
- [x] Run engine tests/format/strict lint; report APIs and RED/GREEN logs.

### Task 2: compatible display settings (settings worker)

Files: services settings.rs/tests; new app appearance.rs (root registers module).
Interfaces: AppSettings ui_scale_percent:u16, map_depth:u8, show_hidden:bool,
sidebar_width:u16, theme accepts system. `appearance::system_dark()->Option<bool>`;
`appearance::resolve(theme:&str, dark:Option<bool>)->&'static str`.
- [x] RED tests: old documents keep defaults, roundtrip fields, invalid ranges,
  explicit themes beat system, missing system preference uses light.
- [x] Implement serde-compatible fields and bounded native read; no registry writes.
- [x] Verify services tests and root-provided app compilation; report evidence.

### Task 3: selected export renderer (export worker)

Files: new app selection_export.rs (root registers module); its unit tests.
Interface: `render_selected(live:&LiveIndex, marked:&HashSet<String>, language:&str)
 -> SelectedExport { list:String, prompt:String, count:usize }`.
Use only indexed extant matches, outermost targets, stable order, decimal GB,
bounded output; escaped names do not become runnable list entries.
- [x] RED tests: overlap, stale/foreign marks, empty selection, newline/markup.
- [x] Render RU/EN list and advisory prompt; no disk/process/clipboard operations.
- [x] Root runs app tests after module registration; worker reports API/limits.

### Task 4: runtime/GPUI/cache integration (root)

Files: contract/runtime/ui/map_view/locale/main/verify, new scan_cache.rs/tests.
- [x] RED runtime/frame tests: settings, age/depth/hidden, sibling navigation,
  widening cancel/failure/reuse, selection export, input isolation and scaled layout.
- [x] Integrate engine projection/settings/provider/export; add localized controls,
  selection details, sidebar adjustment, shortcuts/help and active pointer target.
- [x] Implement bounded identity-checked ordinary cache and above-root worker flow;
  clear on deletion, label reuse, keep snapshot on failed/cancelled widening.
- [x] Verify scoped tests then full workspace/fmt/all-target strict Clippy.
- [ ] Commit changes/version alpha.5; independent immutable-diff review and fixes.
- [ ] Build/check fresh portable, extracted ordinary-user EXE and read-only C proof.
- [ ] Update PR #5 title/body/provenance; confirm hosted CI; disclose manual limits.
