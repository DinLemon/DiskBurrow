# DiskBurrow Rust client

Version **0.3.0-alpha.3**. Rust/GPUI is the primary client in `main`, following
explicit user approval to replace the C# client. This is still an unsigned
preview; merging the source does not complete the manual acceptance checks.
C#/WPF source and its immutable stable release remain available at
[tag v0.2.2](https://github.com/DinLemon/DiskBurrow/tree/v0.2.2) and
[release 0.2.2](https://github.com/DinLemon/DiskBurrow/releases/tag/v0.2.2).

The five crates contain the GPUI application, full live index/map, compatible
settings/history/monitoring services, native identity-bound cleanup, and the
vendored MIT disktree scanner. The main application runs as an ordinary user;
only its separately authenticated read-only NTFS helper requests UAC.

## Build on Windows x64

Install Rust 1.97.0 and Visual Studio 2022 C++ Build Tools with a Windows SDK.
Use an x64 MSVC developer PowerShell. Python 3.10+ is required only for the
packaging notice collector. Keep build files on a drive with sufficient space:

```powershell
$env:CARGO_TARGET_DIR = 'E:\DiskBurrowBuild\debug'
Set-Location rust
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -j 2 -- -D warnings
cargo test --workspace --locked -j 2
Set-Location ..
./scripts/publish-rust.ps1 -OutputDirectory E:\DiskBurrowBuild\alpha3 -TargetDirectory E:\DiskBurrowBuild\release
```

The publisher creates a new directory only, builds with static CRT, checks PE
imports/version, collects the conservative Windows resolved dependency notices,
and packages an allowlist with checksums and source provenance. It never copies
settings, history, build caches or diagnostics into the archive. Some notices
cover build tools or test-unified features rather than shipped runtime code.
Fallback texts in `licenses/` preserve upstream package attribution when crates
omit a license file; their source is recorded in each text. Unmodified dependency
source archives are linked in `ThirdParty/index.json`, including MPL components.

## Validation status

Local Windows checks: **281 tests pass, one upstream timing test ignored**;
strict Clippy and formatting pass. GPUI frame tests cover six pages, live RU/EN,
light/dark, editable GB thresholds, pinned Save and a minimum-size long-path map.
Native fixtures cover reparse/identity races, immutable reviewed plans, private
pipe cancellation, registry rollback and compatible SQLite storage. An actual ordinary-user EXE scans a disposable fixture, produces live map
geometry and exports a compatible snapshot. The alpha.2 archive extracted from
source `90d8091` also completed a read-only C: scan, populated its live map,
saved SQLite history and exported JSON. That full application run took about
41 seconds and peaked at 627,806,208 bytes of working set. Measurements and
limits are documented in [the validation record](../docs/rust-validation.md). A real application window was observed by its
process title/handle after startup.

**Outstanding:** supported manual desktop interaction, real Rust UAC
approval/denial, tray clicks, autostart at logon, and final verification of edits after the independently reviewed alpha.2
source `90d8091`. That review identified an automatic retry issue being fixed;
see [the validation record](../docs/rust-validation.md). The desktop helper failed initialization with `failed to write kernel
assets` / Windows error 3. Programmatic frame tests and window existence do not
verify mouse navigation or visual correctness on the user's desktop.

Use a new explicit `--data-dir` and disposable scan folder for isolated trials;
this disables tray, monitoring and autostart but does not sandbox deliberate
deletion. Normal mode uses `%LOCALAPPDATA%\DiskBurrow`: close 0.2.2 and back up
that directory first. No destructive validation uses real user data.

## Metadata and history limits

Ordinary scans and live indexes reject retained metadata beyond **20 million
entries** or a **1 GiB estimated resident budget**. This is an estimate used by
the scanner, not an OS working-set cap. Alpha.2 counts retained basenames for
ordinary files, full paths for directories/diagnostics, and the root once.
A genuinely exhausted budget publishes no completed result; cancellation and
inaccessible areas are represented explicitly. The MFT helper separately limits
its records and IPC payload.

History retains up to 30 scans and 100 largest files per snapshot, under a
250 MiB shared SQLite/sidecar/log budget (logs at most 10 MiB). Large directory
inventories can cause earlier snapshots to be pruned before reaching 30.
The map uses the full live index; snapshot exports contain bounded largest-file
lists, all retained directory observations and diagnostic issues. Exports may
therefore be much larger than the portable executable.
