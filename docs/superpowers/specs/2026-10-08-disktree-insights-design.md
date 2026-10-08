# DiskBurrow: insights, Git and reclaimed space

Continuation of the user's approved disktree port, following the 2026-10-08 source comparison. This pass implements its first two priority groups: classification/recommendations/Git, and estimated/actual space with post-removal refresh. The remaining navigation, display and export groups remain recorded in the comparison; this pass must not label them completed.

## Outcome

The full live index explains the kinds of data occupying the disk, shows a bounded “Worth a look” list, and describes Git work that might be lost before reviewed deletion. The manual preview estimates physical space recoverable on the displayed volume. The result measures actual volume free-space change and offers refreshed scan data automatically.

## Boundaries

- Windows x64 Rust/GPUI; reuse the existing five-crate architecture and vendored disktree source/license.
- Permanent deletion remains separately reviewed and confirmed. Recommendations never authorize deletion and never run a tool's clean command.
- Existing native identity, ancestor, protected-directory, cloud/reparse and cancellation rules remain authoritative.
- Git inspection is read-only, local, bounded and cancellable. No fetch, hooks, fsmonitor, pager, filter execution, terminal prompts or configuration inherited from an untrusted environment. Missing Git, unsafe repository configuration, timeout and errors mean unknown, never clean.
- Dirty/stash/unpushed/unknown Git information is disclosed in the selection/review; it is advisory and does not replace native revalidation. Repo inspection is repeated when preparing the reviewed inventory.
- Recoverable-byte estimates use known allocation and unique physical identities on the target volume. All identities observed with multiple hardlinks are excluded, including complete selected link sets: the existing executor retains the reviewed link count per name, so its first deletion prevents deletion of a later reviewed name. Unknown allocation is not promised; directories' aggregate sizes are not counted again.
- Actual free-space change is a volume measurement, affected by concurrent programs; negative or unavailable changes are explicit.
- Automatic refresh follows a completed deletion attempt, including partial outcomes, after the deletion worker finishes. Preserve the deletion report across refresh; do not silently discard cancellations/failures or fabricate a completed scan.
- RU/EN and light/dark presentation remain supported. User history/settings schemas stay compatible; additions to transient UI contracts are permitted.
- All build/cache/test writes stay under the owned E: workspace. Destructive tests use only generated fixtures there. Real C: validation is read-only and uses an isolated E: data directory.
- Native desktop automation is unavailable. Programmatic frames, process/SQLite/ZIP proofs are not manual GUI/UAC/tray/logon acceptance.

## Implementation units

1. Engine analytics: use the existing category metadata and vendored classification vocabulary. Derive inherited reclaim reasons from indexed names/siblings without new filesystem reads. Produce bounded nonoverlapping recommendation roots with conservative coverage information.
2. Git inspection: a UI-independent application module returns clean/dirty/stash/ahead/unknown information. Work is executed on background operation workers, including during manual preview; frozen results are published only for the current generation.
3. Native reclaim: a pure estimator over the reviewed native file inventory, plus same-volume before/after measurement during execution. Existing deletion authority remains unchanged.
4. UI/runtime integration: category colors/legend and separate hatching for reconstructable data; a recommendation list and Git detail/review messages; estimated/actual space disclosures and a post-delete refresh that retains the operation report.

## Verification

Use real generated Git repositories for clean/dirty/stash/ahead/unsafe-config/no-upstream states, plus bounded-process cancellation checks. Native fixtures cover overlapping roots, internal/external hardlinks, unknown allocation and unavailable/negative free-space observations. Engine fixtures cover inherited classification and nonoverlapping recommendations. GPUI/runtime checks cover RU/EN, both appearances, stale worker publications and retained deletion outcomes after scan refresh. Finish with formatting, strict lint, the locked workspace suite, source review and a freshly extracted portable executable fixture check.
