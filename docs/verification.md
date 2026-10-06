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

The publisher accepts a version as a validated argument, requires a new output directory, rejects reparse ancestors, and never recursively deletes any directory. It consumes MSBuild `ResolvedFileToPublish`, checks required native/runtime files and both `includedFrameworks` versions, omits PDBs and forbids databases/logs/reports/test trees. The ZIP includes exactly that publish list plus `LICENSE`, `GETTING-STARTED.txt` and `PACKAGE-MANIFEST.json`; SHA-256 names the corresponding archive. Compiler paths are normalized to `/_/src/`, RepositoryUrl is the approved HTTPS project URL, and packaged byte streams are scanned for local machine paths in UTF-8 and both UTF-16 alignments. Local runtime proofs never enter the package.

Local package tests cover exact ZIP entries against the separate publish manifest, checksum, absence of local history/reports/test trees/PDBs and local paths in shipped binaries, README rules, rejection of existing output without altering a sentinel, rejection of a version containing command syntax, embedded locale resource names and approved links. Runtime verification is described separately below. CI scripts have been parsed locally; hosted Actions, public repository ownership/name recheck, actual release upload and downloadable assets await controller publication after independent reviews. No hosted result is claimed here.

## Actual portable executable evidence (2026-10-06)

Final local Release verification: locked solution restore succeeded; Release solution build had **0 warnings / 0 errors**. Package checks passed **7/7** in the full run. The full Release suite passed **225**, skipped **1**, failed **0** (226 total). The existing skip is `CleanupRaceTests.FileReplacedBySymlinkIsSkipped`: Windows symlink privilege was unavailable (error 1314), so this actual symlink replacement case remains unverified on the host.

The actual ZIP has **483 allowlisted entries**, including bundled `coreclr.dll`, `hostpolicy.dll`, WindowsDesktop assemblies, native `e_sqlite3.dll`, the app's own embedded icon/locales, runtime/dependency notices and the three supplemental files. `DiskBurrow.exe` was extracted into an independent ignored work directory and run only as `--verify-runtime` against a separately marked owned workspace. The same PowerShell smoke block in CI was executed locally. It finished with exit **0** while both DOTNET_ROOT variables pointed to an empty directory and DOTNET_MULTILEVEL_LOOKUP was 0. These variables alone are not the proof: the recorded loaded CoreCLR, hostpolicy and native SQLite modules all resolved inside the extracted package. Environment.Version was **10.0.12**, and runtimeconfig listed **Microsoft.NETCore.App 10.0.12** and **Microsoft.WindowsDesktop.App 10.0.12**. Bundled WindowsDesktop and CoreCLR file product versions were checked separately.

The proof constructed the real WPF window and NotifyIcon, observed Dispatcher publication of one native fixture file, loaded one real SQLite history snapshot, round-tripped settings, and switched the actual RU/EN resources after the executable rename. Its owned data directory was outside the EXE directory. MonitoringStarted, CleanupExecuted, AutostartApplied and VisualInteractionVerified were all **false**. Full raw module/data paths and logs are private ignored work files and are not shipped.

This programmatic run does **not** verify visual layout or user clicks. All previously listed GUI/tray/dialog/cancel/Windows-session-ending/cleanup/autostart manual limitations remain. The GitHub/Releases buttons now use the approved project URLs, but the actual user-click navigation and the live external destination remain pending publication/manual review. No normal monitoring process, real-user cleanup, autostart registration, elevation or background networking was exercised by the smoke.


## Retained-result integration verification

Focused regressions reopen an actual paused app runtime with real SQLite history and an injected scanner that rejects traversal; startup exposes the retained overview/history with zero scanner calls. Empty/corrupt history preserves database bytes and reports a localized recovery route. A held initial history read cannot replace a newer selected root. Fake-clock monitoring confirms fresh rooted/time-stamped volume counters agree with low-space alerts during pause, reject another root's observation, and refresh on activation without scanning.

Retained growth notifications carry root, current and previous snapshot IDs. Tests replace the visible result with another root, activate the old notification, recover its exact retained snapshot/comparison and select a folder outside the top-row projection. Expired snapshots and missing previous comparisons have explicit states. The actual Windows balloon click remains a manual check.

Cleanup report payloads now persist immutable reviewed path/rule/file metadata beside each outcome. A partial fake cleanup is stored, the plan discarded, and the report deserialized through a reopened SQLite connection; legacy GUID-only payloads remain unmapped. Owned native cancellation also preserves audited completed outcomes. Existing budget/journal-failure tests still preserve the live report. A SQLite trigger aborts insertion after retention pruning and proves all prior rows survive reopening. Save/Export status, successful/cancelled reanalysis, locale-key membership and changed-PowerShell-location publisher guards have focused coverage. These tests do not expand deletion authority or prove human interaction.
