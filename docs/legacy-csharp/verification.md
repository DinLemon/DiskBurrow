# Verification

## Disk scanning (Task 1)

Verified on Windows x64 with .NET SDK 10.0.100:

```powershell
dotnet restore --locked-mode
dotnet test
```

Task 1's complete suite passed 23 tests with no skips. Fixtures are confined to the ignored `work/fixtures` directory and removed by their test owner. Tests cover actual NTFS hard links, junction cycles, requested roots below junction or Offline ancestors, temporary denied directory ACLs, paths longer than 260 characters, sparse and compressed allocation, and metadata inspection while content access is blocked by share mode.

Sparse and compressed physical sizes are read through `FileCompressionInfo` and verified against independent `GetCompressedFileSizeW` results. Ordinary files use `FileStandardInfo.AllocationSize`. The scanner has at most one metadata handle open at a time, retains only the 100 largest file observations, and uses a minimal identity/canonical-path map to avoid counting hard-linked allocation twice.

Cloud safety tests inject Offline, RecallOnOpen and RecallOnDataAccess attributes and reject any attempt to open their metadata handles. Real OneDrive provider hydration is not verified. Scanning is a live metadata observation, not an atomic filesystem snapshot; detected changes and inaccessible/skipped entries mark their ancestors incomplete with unknown physical totals.

## Tray and application integration (Task 6)

Task 6 checks distinguish executable/runtime evidence from human interaction evidence. The debug executable ran on the local Windows host using installed .NET 10.0.0. It constructed the actual WPF window and NotifyIcon, published a tiny native fixture scan on the WPF Dispatcher, round-tripped real SQLite history and settings, switched RU/EN resource dictionaries, disposed services and exited with code 0. This does not establish portable/self-contained deployment; Task 7 must verify the published runtime paths.

The verification mode never starts normal monitoring, applies autostart, executes cleanup, opens external links, or uses real user TEMP/browser data. All files it creates are in a new owned child below the supplied `work/` directory. Unknown arguments and an unowned/relative verification directory exit with code 2 without falling back to normal startup.

Example from the repository root (change the EXE path for a published package):

```powershell
$verificationWorkspace = Join-Path (Get-Location) 'work/runtime-verification'
New-Item -ItemType Directory -Force $verificationWorkspace | Out-Null
'DiskBurrow owned runtime verification workspace' | Set-Content -LiteralPath (Join-Path $verificationWorkspace '.diskburrow-runtime-workspace')
& '.\src\DiskBurrow.App\bin\Debug\net10.0-windows\DiskBurrow.exe' --verify-runtime $verificationWorkspace
```

Inspect the new `runtime-<id>/runtime-proof.json`. It records the actual runtime version, executable and loaded CoreCLR/hostpolicy/native SQLite module paths, isolated data directory, Dispatcher/native-scan/storage/resource results, and explicit false flags for monitoring, cleanup, autostart and visual interaction. The workspace needs the exact ownership marker, a non-reparse ancestor chain, and an absolute location under `work/`; each run uses a fresh child outside the EXE directory. A runtime failure writes `runtime-failure.json` in that fresh child and exits 2. Proofs contain local paths and stay in ignored local work; do not include them in releases.

Automated presentation tests cover empty and confirmed selection, filtering and exclusions, partial cleanup/unknown free-space delta, journal failures without repeated deletion, history diagnostics without destroying the overview, explicit/atomic export, live localized status/resources, busy/cancellation coordination including cancellation before scan initialization, off-Dispatcher history/journal calls, ordered snapshot projection, and second-instance local pipe activation. Shutdown tests use a real STA WPF Dispatcher to prove queued UI continuations can publish completed results and continue journaling while a bounded drain pumps messages; a never-finishing operation reaches its bound.

Normal Exit cancels and drains asynchronously before shutdown. Windows session ending cancels work and pumps Dispatcher continuations for at most two seconds, without setting the OS shutdown cancellation flag. If Windows terminates the process beyond that bound, durable cleanup journaling is best effort; completed native outcomes may outlive their journal entry. An unpumped synchronous wait is never used in `OnExit`. Incomplete scans remain excluded from stored history and growth baselines; an earlier completed overview remains visible after cancellation.

## Unverified interaction scenarios

The supported GUI helper could not initialize on this host: `failed to write kernel assets` / path-not-found `os error 3`, reproduced after reset and one retry. Native computer use is disabled. No custom helper protocol or PowerShell UI Automation fallback was used. Consequently the following remain **unverified** by human clicks/screenshots:

- Actual visual layout, focus/keyboard navigation, tray rendering and tray menu interactions.
- Close-to-tray, restore, activation from a second actual EXE, balloon click and pending-notification navigation.
- UI responsiveness on a large real scan, cancel buttons, input validation and both visible localizations.
- Settings dialog persistence through actual clicks, folder chooser and export disclosure/destination dialogs.
- Actual Windows shutdown/session ending and its two-second best-effort durability bound.
- Actual cleanup button/confirmation flow and autostart registration. Development/runtime smoke deliberately does not exercise either on real user data.

Manual review should open the normal EXE, inspect each page in RU and EN, close/restore through the tray, start a second copy, inspect a read-only scan and cancel it, and review candidate selection/filtering/exclusions without executing deletion. Use only owned fixtures for any future destructive test. Keep autostart off during smoke; enabling it or actual cleanup is an explicit user action.

Observed standard NuGet/pip paths are recommendations only; their documents explain supported application tools. DiskBurrow does not execute those tools or add their files to cleanup candidates. Sources checked during implementation: [NuGet cache management](https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders), [pip caching](https://pip.pypa.io/en/stable/topics/caching/), [Chrome desktop browsing data](https://support.google.com/chrome/answer/2392709?co=GENIE.Platform%3DDesktop), [Edge browsing data](https://support.microsoft.com/en-us/edge/view-and-delete-browser-history-in-microsoft-edge). The current-user mutex uses the global namespace to cover Windows sessions, while activation uses a local `CurrentUserOnly` pipe. [Windows kernel object namespaces](https://learn.microsoft.com/en-us/windows/win32/termserv/kernel-object-namespaces).

## Read-only system-volume observation (2026-10-05)

An independent current-user, non-elevated metadata-only probe ran the existing scanner. Scanner/Core/native sources were unchanged afterward; packaging did not repeat this full traversal. No user cleanup, content reads, hydration, attribute/ACL/registry writes or deletion were performed. Only sanitized aggregates were retained for publication.

- Cancellation requested at 750 ms produced `OperationCanceledException` at 763 ms, with no partial snapshot. Last sampled progress: 35 files / 4 directories; these are samples, not final counts.
- The completed traversal took 372,248 ms (6 minutes 12.248 seconds), with 275,307 directory observations: 273,272 coverage-complete and 2,035 incomplete. Last sampled progress: 1,441,911 files / 274,821 directories; exact total file count is not exposed by this snapshot.
- Logical path-visible total: 224,246,060,278 bytes. Root allocation is unknown. The disjoint, canonical known-allocation subtree subtotal was 197,630,486,286 bytes across 24,027 selected subtree roots; this does not establish whole-volume allocation. Logical paths may repeat hard links.
- 5,532 issue observations: AccessDenied 879, ReparseSkipped 449, CloudSkipped 4,198, MetadataUnavailable 0, ChangedDuringScan 2, IoFailure 4. Issue counts are not unique inaccessible-directory counts; coverage gaps propagate upward.
- Separate Windows volume counters: total 248,887,898,112 bytes; before free 2,069,028,864 / used 246,818,869,248, after free 244,383,744 / used 248,643,514,368. Concurrent host activity explains changes during the scan. These counters include unobserved/system-managed space and cannot be equated with scanner totals. No cleanup was attempted.

Traversal completion proves the permitted walk finished, while coverage remained incomplete. Resource spot samples (not peaks) were approximately 131 MiB at 31 CPU seconds and 368 MiB at 166 CPU seconds, with 292 handles in both samples. A million-path scan is not an instantaneous operation; responsiveness and cancel-button interaction on that scale remain manual/unverified.

## Portable release construction

Windows x64 local packaging uses SDK 10.0.100 with the pinned **10.0.12** self-contained CoreCLR and WindowsDesktop runtime packs. `global.json` allows newer 10.0 SDK feature bands so hosted `setup-dotnet` 10.0.x can resolve the SDK. [Official .NET 10 release metadata](https://dotnetcli.blob.core.windows.net/dotnet/release-metadata/10.0/releases.json), checked 2026-10-06, reports runtime 10.0.12 / SDK 10.0.401 / release date 2026-09-08. The SDK's initial 10.0.0 default was not used for the release runtime. No SDK was installed for this task.

The publisher accepts a version as a validated argument, requires a new output directory, rejects reparse ancestors, and never recursively deletes any directory. It consumes MSBuild `ResolvedFileToPublish`, checks required native/runtime files and both `includedFrameworks` versions, omits PDBs and forbids databases/logs/reports/test trees. From 0.2.0, the ZIP includes exactly that publish list plus `LICENSE`, `THIRD_PARTY_NOTICES.txt`, `GETTING-STARTED.txt` and `PACKAGE-MANIFEST.json`; SHA-256 names the corresponding archive. Compiler paths are normalized to `/_/src/`, RepositoryUrl is the approved HTTPS project URL, and packaged byte streams are scanned for local machine paths in UTF-8 and both UTF-16 alignments. Local runtime proofs never enter the package.

Local package tests cover exact ZIP entries against the separate publish manifest, checksum, absence of local history/reports/test trees/PDBs and local paths in shipped binaries, README rules, rejection of existing output without altering a sentinel, rejection of a version containing command syntax, embedded locale resource names and approved links. Runtime and hosted verification are version-specific; the evidence below distinguishes completed checks from manual interaction gaps.

## 0.1.1 portable executable evidence (2026-10-06)

Final local Release verification: locked solution restore succeeded; Release solution build had **0 warnings / 0 errors**. Package checks passed **7/7** in the full run. The full Release suite passed **225**, skipped **1**, failed **0** (226 total). The existing skip is `CleanupRaceTests.FileReplacedBySymlinkIsSkipped`: Windows symlink privilege was unavailable (error 1314), so this actual symlink replacement case remains unverified on the host.

The actual ZIP has **483 allowlisted entries**, including bundled `coreclr.dll`, `hostpolicy.dll`, WindowsDesktop assemblies, native `e_sqlite3.dll`, the app's own embedded icon/locales, runtime/dependency notices and the three supplemental files. `DiskBurrow.exe` was extracted into an independent ignored work directory and run only as `--verify-runtime` against a separately marked owned workspace. The same PowerShell smoke block in CI was executed locally. It finished with exit **0** while both DOTNET_ROOT variables pointed to an empty directory and DOTNET_MULTILEVEL_LOOKUP was 0. These variables alone are not the proof: the recorded loaded CoreCLR, hostpolicy and native SQLite modules all resolved inside the extracted package. Environment.Version was **10.0.12**, and runtimeconfig listed **Microsoft.NETCore.App 10.0.12** and **Microsoft.WindowsDesktop.App 10.0.12**. Bundled WindowsDesktop and CoreCLR file product versions were checked separately.

The proof constructed the real WPF window and NotifyIcon, observed Dispatcher publication of one native fixture file, loaded one real SQLite history snapshot, round-tripped settings, and switched the actual RU/EN resources after the executable rename. Its owned data directory was outside the EXE directory. MonitoringStarted, CleanupExecuted, AutostartApplied and VisualInteractionVerified were all **false**. Full raw module/data paths and logs are private ignored work files and are not shipped.

This programmatic run does **not** verify visual layout or user clicks. All previously listed GUI/tray/dialog/cancel/Windows-session-ending/cleanup/autostart manual limitations remain. The GitHub/Releases buttons now use the approved project URLs, but the actual user-click navigation and the live external destination remain pending publication/manual review. No normal monitoring process, real-user cleanup, autostart registration, elevation or background networking was exercised by the smoke.


## Retained-result integration verification

Focused regressions reopen an actual paused app runtime with real SQLite history and an injected scanner that rejects traversal; startup exposes the retained overview/history with zero scanner calls. Empty/corrupt history preserves database bytes and reports a localized recovery route. A held initial history read cannot replace a newer selected root. Fake-clock monitoring confirms fresh rooted/time-stamped volume counters agree with low-space alerts during pause, reject another root's observation, and refresh on activation without scanning.

Retained growth notifications carry root, current and previous snapshot IDs. Tests replace the visible result with another root, activate the old notification, recover its exact retained snapshot/comparison and select a folder outside the top-row projection. Expired snapshots and missing previous comparisons have explicit states. The actual Windows balloon click remains a manual check.

Cleanup report payloads now persist immutable reviewed path/rule/file metadata beside each outcome. A partial fake cleanup is stored, the plan discarded, and the report deserialized through a reopened SQLite connection; legacy GUID-only payloads remain unmapped. Owned native cancellation also preserves audited completed outcomes. Existing budget/journal-failure tests still preserve the live report. A SQLite trigger aborts insertion after retention pruning and proves all prior rows survive reopening. Save/Export status, successful/cancelled reanalysis, locale-key membership and changed-PowerShell-location publisher guards have focused coverage. These tests do not expand deletion authority or prove human interaction.

## 0.2.0 local candidate verification (2026-10-06)

Locked restore and Release build completed with **0 warnings / 0 errors**. The complete suite passed **334**, skipped **1**, failed **0** (335 total), including all seven independent-package checks. The skip remains the actual Windows symlink replacement test requiring privilege 1314. Independent reviews found and closed defects in protected-path handling, ancestor replacement, live report availability, search draining, keyboard focus, selection normalization and MFT parsing/coverage/protocol capacity.

Actual WPF controls were rendered in RU/EN at normal and minimum window sizes. Visible DataGrid headers switch with the locale without discarding rows or selections; Settings controls have compact measured spacing. Owned native fixtures verify the new permanent-delete service and separate confirmation, stale/changed plans, partial reports and local SQLite audit. Real WPF focus traversal leaves the map with Next/Previous. These are programmatic checks, separate from the human interaction limitations above.

An independently extracted self-contained candidate ran the real `runas` helper for **C:\**, from an **unelevated** parent with the .NET **10.0.12** runtime and native SQLite loaded from that package. The actual authenticated elevated helper finished with exit **0** and both processes exited. It produced **1,500,152** live entries: **1,226,260** file paths and **273,892** directories. Backend reading/building took **19.209 seconds**; the helper operation took **21.267 seconds**, and transfer plus map projection completed in **29.615 seconds**. Eight accessible largest-file samples matched an independent native query for identity, logical size and modification time; two other samples were inaccessible. The GUI's observed peak working set was **1,425,346,560 bytes**, approximately **1.33 GiB**. Metadata budgets are estimated capacity guards, not process working-set caps.

The observation reported **2,415** reparse skips, **73,936** cloud skips and **20** unavailable-metadata observations. It is a live, non-atomic MFT observation with coverage gaps, not a claim that every path is accessible or that scanner totals equal Windows used-space counters. MFT physical sizes include named data streams, unlike the walker's main-stream measurement. The previous ordinary walk was at another time and under different permissions, so these timings are not a controlled speedup comparison. No user files were deleted, hydrated or read as content; raw MFT chunks necessarily contain resident metadata/data bytes, which are not exported as file contents. Monitoring and autostart stayed disabled; only owned work directories received the proof and history.

The first unbuffered result-transfer experiment was stopped without a snapshot; its elevated child exited after parent termination. A behavioral regression demonstrated 340,043 underlying writes for 20,000 entries. Direction-specific 64 KiB buffers reduced both read and write calls below 200 without changing authenticated handshake, bounds or payload semantics. The completed timing above uses the corrected pipeline.

To repeat the optional real-volume verifier, use the owned absolute `work/` workspace and exact marker described above, then run `DiskBurrow.exe --verify-runtime <workspace> --mft-root C:\`. This requests UAC, requires the same Windows account, and creates isolated history only under a new owned run directory. The default verifier does not elevate or read a user volume. It verifies a live map as well as the native fixture/runtime/storage/resources. Hosted CI executes this default mode, checks that no MFT helper ran, and requires successful validation before attaching release assets. Release notes identify hosted/downloaded-artifact checks separately from this local candidate benchmark. Actual human clicks on the fast-scan and deletion buttons, UAC rejection, tray interactions and OS shutdown remain unverified.

## 0.2.1: decimal units, appearance and actual UI validation

Sizes and editable thresholds use decimal GB (1,000,000,000 bytes). Saved byte thresholds keep their effective value, including legacy documents with omitted thresholds; new installations default to 15 GB / 5 GB. Light/dark appearance previews immediately and persists on explicit Save. Window, navigation, fields, table headers/rows, dropdowns, tooltips and the read-only manual review share semantic palette resources. Settings Save remains visible outside the scrolling form.

The outer map ScrollViewer previously measured its star row with unbounded height. With a long owned fixture path at 880x600, only about 55 pixels of the map were initially visible. The corrected view keeps its canvas fully inside the page: roughly 200 pixels in Russian and 212 in English, about 430 at 1280x800. Tables retain usable internal viewports, empty alert fields no longer reserve 56 pixels, and measured nonwrapping headers fit both languages. An isolated real WPF/native-fixture probe checked populated views for both sizes, languages and palettes; the regression suite also rendered 48 theme/language/size combinations and checked derived-window palette changes, field contrast and dropdown resources.

Fresh local final-source suite: **352 passed, 1 skipped, 353 total**, Release build with zero warnings/errors. The existing native symlink-replacement case was skipped because this host lacks symlink privilege; the skip is not a passing test. The desktop checks ran the ordinary locally built EXE through the supported Computer Use helper: all six pages in RU/dark and EN/light, a real ordinary scan over an owned fixture, read-only manual review, declining the separate permanent-deletion confirmation, Settings Save, cancelling a scheduled ordinary scan, close-to-hidden and restoring the same process by a second launch. No user files were deleted; the destructive native service tests use their own generated fixtures.

Actual desktop clicks exposed a preexisting scan-publication failure missed by direct UI-thread model probes. The worker's SnapshotChanged event entered BeginDisplay and selection refresh before its first await; DiskMapControl.ModelUpdated accessed a DispatcherObject off-thread. The snapshot event now begins on the UI dispatcher and rechecks the selected root and shutdown state. A real WPF scan-button regression failed on Map, Largest and Overview before the fix and passes on all three afterward.

The main thread's Computer Use kernel still fails before initialization with Windows error 3 while an isolated subagent's supported helper works. This is a tool-session failure, not an established DiskBurrow fault; its missing path is unknown. Actual native resizing to 880x600, tray-icon/menu clicks, denial on the secure UAC desktop and actual Windows-logon autostart remain unverified. Minimum-size rendering is a programmatic WPF result; UAC handling/autostart retain their separate existing controlled tests. No UI automation was used to act on security permission prompts or to approve irreversible deletion.

## v0.2.2 empty-table caption regression

The final native GUI check of v0.2.1 found the Russian Cleanup selection caption clipped in a narrow empty table. The real 880 x 600 WPF window reproduces this: the 52 px header positions its 50.34 px label 6 px from the left, so the label extends beyond the column. The regression checks the transformed label bounds as well as text width. The shorter Russian caption `Выбор` fits. v0.2.1 and its published assets remain immutable.

Final v0.2.2 local Release suite: 352 passed, 1 existing symlink-privilege skip, 353 total; all 48 WPF theme/language/size views passed, including the empty Cleanup caption bounds. Native GUI checks and downloaded-package results are recorded separately from these programmatic checks.
