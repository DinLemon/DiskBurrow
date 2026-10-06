# DiskBurrow

Portable disk monitoring and careful, explicit cleanup for Windows. [Русский](README.ru.md).
Source: [DinLemon/DiskBurrow](https://github.com/DinLemon/DiskBurrow). Downloads: [Releases](https://github.com/DinLemon/DiskBurrow/releases).

## Install and run

1. Download `DiskBurrow-0.1.0-win-x64.zip` and its `.sha256` from Releases. Verify with `Get-FileHash .\DiskBurrow-0.1.0-win-x64.zip -Algorithm SHA256` and compare the complete hash.
2. Extract the **whole** ZIP into a folder; keep its files together. Run `DiskBurrow.exe` as your ordinary user. Requires Windows 10 22H2 or Windows 11 x64; .NET 10.0.12 and native SQLite are bundled. No installer, service or elevation is required.
3. Russian is the default; switch to English in Settings. Closing the window hides it to the tray. Tray **Exit** cancels/drains work and ends monitoring. A second copy activates the first instance for your user.

The build is **unsigned**; Windows may show an unknown-publisher/SmartScreen warning. Review the source and checksum before deciding to run it. A checksum checks integrity; it is not a publisher signature.

## Monitoring and privacy

- Windows identifies the system volume. Other local folders/drives can be selected manually; network drives are not monitored in the background.
- Full scans default to every **6 hours**, with a **5 minute** startup delay. Settings allow 1, 6, 12 or 24 hours. Free space is checked every 5 minutes; default low-space warning is below **15 GiB**, folder growth warning at **5 GiB**, at most once per folder per day. Missed scans coalesce; scans never overlap. Full background scans wait on battery by default.
- Autostart is **off** until explicitly enabled and saved. If you move an opted-in installation, review/save the autostart setting at the new location. Pause stops full background scans; free-space checks and low-space alerts continue; Exit stops the app.
- History keeps the latest **30** completed scans and at most **100** largest files per scan. SQLite database, sidecars and logs have a **250 MiB** budget; logs are capped at **10 MiB**. Storage failure leaves the live result visible. Data/settings live under `%LOCALAPPDATA%\DiskBurrow`, outside the EXE folder. Exit before backing up/removing local data; history is not part of the ZIP.
- Cleanup journal records keep reviewed file paths, rule metadata and outcomes locally within the same storage budget. Older GUID-only records are explicitly unmapped; no path is invented.
- No telemetry or automatic network requests. GitHub/Releases and supported maintenance instructions open only on click. An explicitly requested JSON export contains real paths: review the disclosure and destination before sharing it. Deleting the ZIP does not erase local history.

## Cleanup rules

**Deletion is permanent, with no Recycle Bin or undo.** Analyze first, inspect paths/age/reason, select individual files, then confirm. Nothing is selected initially. Exclude files/categories in the preview, or absolute paths in Settings. Selection survives filtering; hidden selected files still count in the selection summary. Reanalyze after changing exclusions. Changed, busy, unverified or disallowed candidates are skipped; partial outcomes remain visible.

- User TEMP: files strictly older than **7 days**, from a safe recognized user temp root. A redirected TEMP hint is not trusted automatically; a narrow custom root must pass approval in Settings.
- User `CrashDumps`: `.dmp` files strictly older than **7 days**.
- Chrome/Edge HTTP `Cache` / `Cache_Data` only, when the corresponding browser is closed. Browser state that cannot be verified blocks that rule. Cookies, passwords, history, service-worker storage and downloads are not cleanup targets. Browser caches have no age threshold.
- Reparse points and cloud placeholders are skipped and never hydrated. The scanner avoids double-charging hard-linked physical allocation; logical path sizes may still repeat. Cleanup may remove an explicitly selected hard-link path after normal revalidation; other links remain, so deletion may release no physical space. Inaccessible/changing entries make coverage incomplete and physical totals unknown. Logical size, known allocation subtotals and Windows used-space counters are different measurements. A completed traversal does not mean complete volume coverage.

NuGet/pip cache suggestions only link to supported program tools; DiskBurrow never executes them or automatically deletes those caches. UI bounds are 1,000 directories, 100 largest files and 2,000 cleanup rows; summaries identify truncation and the full completed snapshot remains available for export/history.

## Recovering corrupt history

1. Use **Exit** in the tray and wait for DiskBurrow to stop. Preserve a copy of `%LOCALAPPDATA%\DiskBurrow` outside that directory, preferably on another disk with space, before changing files.
2. In that directory, rename only `history.db` (for example, `history.db.corrupt-backup`) and any existing `history.db-journal`, `history.db-wal`, `history.db-shm` with the same backup suffix. Move these renamed database/sidecar backups together to the preserved location outside `%LOCALAPPDATA%\DiskBurrow` before restarting: all files left in the data directory count toward the storage budget. Do not delete them or rename unrelated files.
3. Leave `settings.json` and its settings backups unchanged. Restart DiskBurrow; a new history database will be created on the next completed scan or cleanup journal write. Old history is not automatically recovered. Retain the backup for diagnosis.

## Build and verification

Install the .NET 10 SDK (minimum 10.0.100; `global.json` permits newer 10.0 feature bands) and PowerShell 7 on Windows x64:

```powershell
dotnet restore DiskBurrow.slnx --locked-mode
dotnet build DiskBurrow.slnx -c Release --no-restore
dotnet test tests/DiskBurrow.Tests -c Release --no-restore
./scripts/publish.ps1 -Version 0.1.0 -OutputDirectory ./artifacts/release-0.1.0
```

The output directory must be **new**; the publisher never removes/reuses an existing folder. It restores locked dependencies, pins both bundled frameworks to 10.0.12, publishes self-contained win-x64 and zips only the MSBuild publish manifest plus LICENSE/instructions/package manifest. Debug symbols, local scans, reports and tests are excluded; shipped bytes are checked for local source paths. Package tests build an independent fixture package in ignored `work/` and validate manifest/checksum/privacy. NuGet runtime packs require network access during the first build; ordinary app use does not.

Windows CI repeats locked restore, Release build/tests and packaging; branch runs upload artifacts, version-tag runs attach ZIP/checksum to a GitHub Release. **Hosted CI/release download verification is pending external publication.** See [actual evidence and manual checks](docs/verification.md). Programmatic WPF/SQLite/runtime checks do not prove visual layout, tray clicks, dialogs, actual OS shutdown or the real cleanup/autostart click flows. Normal Exit drains; OS-forced termination after the bounded session-ending wait may prevent the last journal write. Only owned fixtures may be used for destructive verification.

MIT © 2026 DinLemon. Runtime/dependency components retain their upstream licenses.
