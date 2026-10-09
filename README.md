<div align="center">

<h1>DiskBurrow</h1>
<p><strong>Find where your disk space disappears.</strong></p>

<p>
  <img src="docs/assets/diskburrow-logo.png" alt="DiskBurrow — the app's drive and burrow emblem, in blue" width="760">
</p>

<p>Track folder growth, keep local history, and review files before cleanup.</p>

<p>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-265D9F?style=flat-square" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-265D9F?style=flat-square" alt="Windows 10 / 11 x64">
  <a href="https://github.com/DinLemon/DiskBurrow/actions/workflows/rust.yml"><img src="https://github.com/DinLemon/DiskBurrow/actions/workflows/rust.yml/badge.svg" alt="Rust validation and portable package status"></a>
  <a href="https://github.com/DinLemon/DiskBurrow/releases/tag/v0.3.0-alpha.3"><img src="https://img.shields.io/badge/Rust-0.3.0--alpha.3-265D9F?style=flat-square" alt="Rust preview version"></a>
</p>

<p>
  <a href="https://github.com/DinLemon/DiskBurrow/releases/tag/v0.3.0-alpha.3"><strong>Download for Windows</strong></a>
  &nbsp; · &nbsp; <a href="#install-and-run">Quick start</a>
  &nbsp; · &nbsp; <a href="README.ru.md">Русский</a>
</p>

<p>Portable. No telemetry. Autostart is your choice.</p>

</div>

---

## Rust is the primary client

The native Rust/GPUI development version is **0.3.0-alpha.5**.
The published Rust preview remains [0.3.0-alpha.3](https://github.com/DinLemon/DiskBurrow/releases/tag/v0.3.0-alpha.3) until a new prerelease is published.
It remains an **unsigned preview**. Manual desktop, UAC, tray and logon checks
are pending: see [validated evidence and limitations](docs/rust-validation.md).
The C#/WPF source is preserved in [tag v0.2.2](https://github.com/DinLemon/DiskBurrow/tree/v0.2.2);
the previous [stable 0.2.2 release](https://github.com/DinLemon/DiskBurrow/releases/tag/v0.2.2) remains available unchanged.

## Install and run

1. Open the published [0.3.0-alpha.3 prerelease page](https://github.com/DinLemon/DiskBurrow/releases/tag/v0.3.0-alpha.3). Its assets are `DiskBurrow-0.3.0-alpha.3-rust-win-x64.zip` and `.zip.sha256`. Alpha.5 features below belong to the development candidate.
2. Verify with `Get-FileHash .\DiskBurrow-0.3.0-alpha.3-rust-win-x64.zip -Algorithm SHA256` and compare the complete hash. Extract the **whole** ZIP into a new folder. Run `DiskBurrow.exe` as your ordinary user. Requires Windows 10 22H2 or Windows 11 x64; installing .NET or Rust is unnecessary. SQLite and the CRT are included in the native build.
3. Close the old client and back up `%LOCALAPPDATA%\DiskBurrow` outside that directory before the first normal launch. Rust uses compatible settings/history. For an isolated trial: `DiskBurrow.exe --data-dir E:\DiskBurrowTrial --scan-root E:\MyTestFolder`. This disables monitoring, tray and autostart; it does **not** sandbox deliberately confirmed deletion.
4. Russian is the default; select English in Settings. Light/dark appearances switch immediately; **Save** persists your choice. In normal mode, closing hides the window to the tray; **Exit** cancels work and ends the app. A second launch activates the existing instance for your user.

The build is **unsigned**; Windows may show an unknown-publisher/SmartScreen warning. A checksum checks integrity; it is not a publisher signature. The optional fast NTFS scan requests UAC only for a separate process that reads metadata.

## Monitoring and privacy

- Windows identifies the system volume. Other local folders/drives can be selected manually; network drives are not monitored in the background.
- Full scans default to every **6 hours**, with a **5 minute** startup delay. Settings allow 1, 6, 12 or 24 hours. Free space is checked every 5 minutes; default low-space warning is below **15 GB**, folder growth warning at **5 GB**, at most once per folder per day. Missed scans coalesce; scans never overlap. Full background scans wait on battery by default.
- Sizes and editable thresholds use decimal **GB** (**1 GB = 1,000,000,000 bytes**). Existing settings retain their exact byte thresholds; the previous 15 GiB / 5 GiB defaults appear as 16.10612736 GB / 5.36870912 GB. Saving without editing preserves those thresholds. JSON exports retain exact byte counts.
- Autostart is **off** until explicitly enabled and saved. If you move an opted-in installation, review/save the autostart setting at the new location. Pause stops full background scans; free-space checks and low-space alerts continue; Exit stops the app.
- History keeps up to **30** completed scans and at most **100** largest files per scan; the storage budget can reduce retention. SQLite database, sidecars and logs have a **250 MiB** budget; logs are capped at **10 MiB**. Storage failure leaves the live result visible. Data/settings live under `%LOCALAPPDATA%\DiskBurrow`, outside the EXE folder. Exit before backing up/removing local data; history is not part of the ZIP.
- Cleanup journal records keep reviewed file paths, rule metadata and outcomes locally within the same storage budget. Older GUID-only records are explicitly unmapped; no path is invented.
- No telemetry or automatic network requests. GitHub/Releases and supported maintenance instructions open only on click. An explicitly requested JSON export contains real paths: review the disclosure and destination before sharing it. Deleting the ZIP does not erase local history.

## Cleanup rules

**Deletion is permanent, with no Recycle Bin or undo.** Analyze first, inspect paths/age/reason, select individual files, then confirm. Nothing is selected initially. Exclude files/categories in the preview, or absolute paths in Settings. Selection survives filtering; hidden selected files still count in the selection summary. Reanalyze after changing exclusions. Changed, busy, unverified or disallowed candidates are skipped; partial outcomes remain visible.

- User TEMP: files strictly older than **7 days**, from a safe recognized user temp root. A redirected TEMP hint is not trusted automatically; a narrow custom root must pass approval in Settings.
- User `CrashDumps`: `.dmp` files strictly older than **7 days**.
- Chrome/Edge HTTP `Cache` / `Cache_Data` only, when the corresponding browser is closed. Browser state that cannot be verified blocks that rule. Cookies, passwords, history, service-worker storage and downloads are not cleanup targets. Browser caches have no age threshold.
- Reparse points and cloud placeholders are skipped and never hydrated. The scanner avoids double-charging hard-linked physical allocation; logical path sizes may still repeat. Cleanup may remove an explicitly selected hard-link path after normal revalidation; other links remain, so deletion may release no physical space. Inaccessible/changing entries make coverage incomplete and physical totals unknown. Logical size, known allocation subtotals and Windows used-space counters are different measurements. A completed traversal does not mean complete volume coverage.

NuGet/pip cache suggestions only link to supported program tools; DiskBurrow never executes them or automatically deletes those caches. The Largest tables show up to 2,000 directories and 100 files; Cleanup shows up to 2,000 candidates. Summaries identify truncation. The disk map uses a separate full live index of observed files; retained history and JSON snapshot exports preserve the existing bounded file list.

## Disk map and fast NTFS scan

The map shows nested folders and files, with area based on size and colors by file type. Navigate into a folder, use breadcrumbs or back/forward/up, zoom with the wheel or keyboard, and filter names with highlighting or isolation. Select a local volume and inspect its used/free counters. Historical snapshots without a full live index are explicitly limited.

Fast scan is an explicit operation for a whole local NTFS volume. Accepting UAC starts a separate read-only MFT helper; background monitoring and cleanup remain in the ordinary user process. Cancelling UAC cancels the request. Unsupported formats or damaged metadata produce an error; use the ordinary scan to inspect a local folder or another filesystem. The MFT of a running volume is not an atomic snapshot and can lag recent changes. A deletion always requires a fresh native preview.

The UAC helper must run as the same Windows account. Entering another administrator account's credentials is unsupported and fails without scanning. The parser and pipe enforce record, payload and estimated metadata memory bounds; exceptionally large volumes can exceed these limits and require an ordinary scan. MFT physical totals include named data streams; the ordinary walker measures the main stream, so their totals can differ.

## Delete selected folders and files

Mark individual folders/files in Largest or the disk map, then review the selected roots, file/folder counts, data size and warnings. Selecting a parent includes its children once. Permanent deletion requires a separate confirmation; cancelling it leaves the files intact. Changing the selection, scan root or snapshot invalidates the preview.

This manual operation is separate from the four cache cleanup rules. Volume roots, system locations, broad user containers and the application's own files/data are protected. Unresolved protected boundaries, inaccessible entries, links and cloud placeholders block a complete deletion plan. Changed, busy or newly created entries are preserved; the result and local journal include partial failures and cancellation. Deletion does not inherit administrator rights from the scan helper.

In the development candidate, review also shows known physical reclaim on the scanned volume. Unknown allocation, other volumes and all files with multiple hardlinks are excluded from the estimate. The measured free-space change can be negative or unavailable and includes activity by other programs. After an attempt, the app rescans the previous scan root and retains the deletion report/journal even if refresh fails or is cancelled.

## Insights and Git checks — alpha.5 development

- Category colors identify code, agent scratch, toolchains, synced data, Git stores, media, documents and caches. Hatching marks potentially reclaimable data; it does not mark files for deletion.
- Cleanup → **Worth a look** lists up to 100 nonoverlapping indexed cache/build/worktree/old-experiment hints of at least 64 MiB. Incomplete coverage/allocation is disclosed. Click a path to inspect it on the map. Existing NuGet/pip hints and maintenance instructions remain under **Observed caches**.
- Selecting a checkout root inspects tracked changes, untracked files, stash and commits ahead of its locally known upstream. The app never fetches. Missing upstream, unsafe configuration, missing Git, timeout and errors are explicitly unknown. Child folders are not treated as their ancestor repository.
- Git is discovered only in OS-known `Program Files\Git`, `Program Files (x86)\Git` or `%LOCALAPPDATA%\Programs\Git` installation directories. Arbitrary PATH entries and portable/custom installations elsewhere are not executed; those configurations report Git unavailable.
- Manual review repeats Git inspection for at most 8 selected roots, before separate permanent confirmation. These counters are advisory and can become stale; native file/ancestor revalidation remains authoritative. Git processes have a 5-second total inspection limit and a 1 MiB combined output budget, with external subprocesses refused and cancellation terminating their Windows job.

## Navigation and display — alpha.5 development

- Switch category/age colors without changing the selected physical/logical/file-count area metric. Last-write bands are unknown/future, up to 7/30/180/365 days and older; age is not evidence of junk. Choose 1–6 drawn levels and hide dot/Windows hidden/system subtrees from the map, search and current Largest lists. The full retained scan and marks remain intact.
- Breadcrumb menus list siblings ranked by the visible selected metric. Above-root breadcrumbs widen an ordinary scan and focus the previous root. A compatible small ordinary subtree may be reused (32 MiB / 100,000 entries / depth 64); the sidebar labels cached observations. Native root identity and timestamps are checked before/after the traversal; changed roots, fast indexes and deletion attempts do not reuse the cache. Failed/cancelled widening preserves completed results.
- Wheel zoom enters a directory on the next notch after it fills the viewport; zooming out at base scale goes up. Arrows/Tab select tiles/siblings, Space marks, Enter enters, right click opens Explorer. The **Keys** dialog lists search, depth, hidden, age, export and rescan shortcuts. Typing in native editors stays separate from map keys.
- Resize the sidebar by dragging its divider; double click or **Reset sidebar width** restores 225 px. Settings offers system/light/dark appearance and 75–150% interface scale, independent of map zoom. **Save** persists display choices; Ctrl+plus/minus/0 changes/resets interface scale.
- **Save selected TXT** writes outermost indexed selected paths to a new file. **Copy review prompt** copies a localized advisory text with escaped path data. Exports contain private paths; inspect them before sharing. These actions never execute commands or authorize deletion; snapshot JSON remains available.

Windows feature groups above are implemented against pinned disktree `6c8d4ce`; platform-specific macOS monitoring/permissions and upstream Trash behavior are outside this Windows app. Permanent deletion with separate confirmation remains the chosen policy. Manual UAC/tray/logon and desktop visual acceptance is still pending.

Disk map layout and NTFS parsing are adapted from [disktree](https://github.com/tobi/disktree), under MIT; see the [vendored core license](rust/vendor/disktree-core/LICENSE). The portable archive includes full dependency notices in `ThirdParty/`.

## Recovering corrupt history

1. Use **Exit** in the tray and wait for DiskBurrow to stop. Preserve a copy of `%LOCALAPPDATA%\DiskBurrow` outside that directory, preferably on another disk with space, before changing files.
2. In that directory, rename only `history.db` (for example, `history.db.corrupt-backup`) and any existing `history.db-journal`, `history.db-wal`, `history.db-shm` with the same backup suffix. Move these renamed database/sidecar backups together to the preserved location outside `%LOCALAPPDATA%\DiskBurrow` before restarting: all files left in the data directory count toward the storage budget. Do not delete them or rename unrelated files.
3. Leave `settings.json` and its settings backups unchanged. Restart DiskBurrow; a new history database will be created on the next completed scan or cleanup journal write. Old history is not automatically recovered. Retain the backup for diagnosis.

## Build and verification

On Windows x64, install Rust **1.97.0**, Visual Studio 2022 C++ Build Tools,
a Windows SDK and PowerShell. Python **3.10+** is needed to collect packaging
notices. Use an x64 MSVC developer PowerShell and put caches/builds on a drive
with sufficient free space:

```powershell
$env:CARGO_TARGET_DIR = 'E:\DiskBurrowBuild\debug'
Set-Location rust
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -j 2 -- -D warnings
cargo test --workspace --locked -j 2
Set-Location ..
./scripts/publish-rust.ps1 -OutputDirectory E:\DiskBurrowBuild\alpha4 -TargetDirectory E:\DiskBurrowBuild\release
```

The output directory must be **new**. The publisher builds a static-CRT executable, verifies PE dependencies/version, collects dependency notices and packages only allowed files with SHA-256 and source provenance. Settings, history, tests and diagnostics are excluded. Initial Cargo restore needs network access; ordinary app use does not.

[Rust CI](https://github.com/DinLemon/DiskBurrow/actions/workflows/rust.yml) checks formatting, Clippy, tests, packaging and the extracted executable under a disposable standard Windows account. A workflow existing does not assert that the current run passed. [Validation evidence and pending manual checks](docs/rust-validation.md) distinguish programmatic GPUI/SQLite tests from actual desktop clicks, UAC and Windows logon. Only owned fixtures are destructive verification targets.

[Detailed Rust build instructions](rust/README.md). [Archived C# 0.2.2 verification](docs/legacy-csharp/verification.md).

MIT © 2026 DinLemon. Dependencies retain their upstream licenses; the portable archive includes full notices in `ThirdParty/`.
