# DiskBurrow Rust preview

Version **0.3.0-alpha.2**, migration branch `rewrite/rust-gpui`. Stable C#/WPF
0.2.2 remains the published release. No Rust release replaces it yet.

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
./scripts/publish-rust.ps1 -OutputDirectory E:\DiskBurrowBuild\preview-1 -TargetDirectory E:\DiskBurrowBuild\release
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

Local Windows checks: **269 tests pass, one upstream timing test ignored**;
strict Clippy and formatting pass. GPUI frame tests cover six pages, live RU/EN,
light/dark, editable GB thresholds, pinned Save and a minimum-size long-path map.
Native fixtures cover reparse/identity races, immutable reviewed plans, private
pipe cancellation, registry rollback and compatible SQLite storage. An actual
ordinary-user EXE scans a disposable fixture, produces live map geometry and
exports a compatible snapshot. A real application window was observed by its
process title/handle after startup.

**Outstanding:** supported manual desktop interaction, real Rust UAC
approval/denial, tray clicks, autostart at logon, and independent whole-branch
review. The desktop helper failed initialization with `failed to write kernel
assets` / Windows error 3. Programmatic frame tests and window existence do not
verify mouse navigation or visual correctness on the user's desktop.

Use a new explicit `--data-dir` and disposable scan folder for preview trials;
this disables tray, monitoring and autostart but does not sandbox deliberate
deletion. Normal mode uses `%LOCALAPPDATA%\DiskBurrow`: close 0.2.2 and back up
that directory first. No destructive validation uses real user data.
