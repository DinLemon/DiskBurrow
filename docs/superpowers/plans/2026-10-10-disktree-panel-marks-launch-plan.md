# Map panel, hierarchical marks and compatible launch implementation plan

> **For agentic workers:** use executing-plans with isolated parallel modules;
> root integrates and a fresh reviewer checks the final immutable diff.

**Goal:** implement the user's three approved remaining Windows groups.
**Architecture:** pure hierarchical mark and CLI modules; bounded same-user launch
IPC; UI-owned forecast generation with native read-only worker; root owns contract,
runtime and GPUI integration. Existing deletion reviews remain authoritative.
**Tech Stack:** Rust1.97/MSVC, GPUI-kit0.6.6, current Windows APIs/vendored core.
**Spec:** ../specs/2026-10-10-disktree-panel-marks-launch-design.md

## Global constraints

Permanent separate confirmation; local one-volume no-follow/no hydration; RU/EN;
880x600 at75/100/150%; lists2000 roots/100 recommendations; immutable scan UUID;
bounded IPC65536 bytes/timeout3s; same SID and executable; E: fixtures only.
No registry installation, helper/history schema change, merge or release.

## Review focus

1. A covered child must never look excluded while still deleted by its parent.
2. Forecasts must not reuse stale marks/volume/index or create deletion authority.
3. CLI/IPC must reject foreign peers, malformed/big payloads and unsafe paths.
4. Editor and confirmation keys must not trigger navigation/exit/deletion.
5. Extra sidebar content must remain reachable without collapsing the map.

### Task1: pure marks (worker)
Files: new app mark_selection.rs. Interface toggle(&mut HashSet<String>, &str)
-> MarkChange {Added{absorbed},Removed,Covered{ancestor},Invalid}; ancestor lookup.
- [ ] RED parent absorbs children, covered child refused, siblings/case/drive bounds.
- [ ] Implement component-aware normalization; report focused RED/GREEN evidence.

### Task2: compatible CLI (worker)
Files: new app cli.rs. Pure parse + LaunchOverrides/worker policy; root registers.
- [ ] RED positional/Unicode/relative/--, conflicts/ranges/legacy/safety flags.
- [ ] Implement all compatible display/worker options and explicit unsupported errors.
- [ ] Verify pure tests; document root-facing API; no entrypoint/runtime edits.

### Task3: same-user launch IPC (worker)
Files: new app launch_ipc.rs, minimal platform integration if agreed with root.
- [ ] RED real owned server/client, peer mismatch, size/timeout/cancel/invalid payload.
- [ ] Implement bounded no-shell native transport with same-SID/executable identity.
- [ ] Report native checks/API; no tray/runtime/main edits without ownership transfer.

### Task4: runtime/map UI/entrypoint (root)
Files: contract/runtime/ui/map_view/locale/main/verify and focused new tests.
- [ ] RED runtime mark hierarchy, worker forecast guards, launch options/queue.
- [ ] Register modules; integrate marked list/recommendations/conditional forecast.
- [ ] RED/implement precise supported map/review shortcuts and editor/modal isolation.
- [ ] RED real minimum-size sidebar frames, covered-parent actions and native launch.
- [ ] Full suite/fmt/strict lint; versionalpha.6; immutable review and fixes.
- [ ] Fresh portable/extracted-EXE proof, PR5 update and current-head hosted CI.
