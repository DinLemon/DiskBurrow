[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$TargetDirectory,
    [string]$Python = 'python',
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$output = [IO.Path]::GetFullPath($OutputDirectory)
$target = [IO.Path]::GetFullPath($TargetDirectory)
if (Test-Path -LiteralPath $output) { throw 'Use a new output directory; existing artifacts are never replaced.' }
$version = '0.3.0-alpha.6'
$oldFlags = $env:RUSTFLAGS
$oldTarget = $env:CARGO_TARGET_DIR
try {
    $env:CARGO_TARGET_DIR = $target
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    Push-Location (Join-Path $repo 'rust')
    try {
        if (-not $SkipBuild) {
            cargo build -p diskburrow-app --release --locked -j 2
            if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' }
        }
        $metadataText = cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc
        if ($LASTEXITCODE -ne 0) { throw 'Cargo dependency inventory failed.' }
    } finally { Pop-Location }
    $exe = Join-Path $target 'release\DiskBurrow.exe'
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Release executable missing.' }
    if ((Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -ne $version) { throw 'Unexpected executable version.' }
    $imports = & dumpbin /nologo /dependents $exe
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect PE imports; run in the MSVC x64 developer shell.' }
    if (($imports -join "`n") -match '(?i)(VCRUNTIME|MSVCP|coreclr|hostpolicy|e_sqlite3)') { throw 'Unexpected external compiler or managed runtime dependency.' }
    $dependencies = @($imports | ForEach-Object { if ($_ -match '^\s+([\w.-]+\.dll)\s*$') { $Matches[1] } })
    $stage = Join-Path $output 'package'
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    $metadataFile = Join-Path $output 'cargo-metadata.private.json'
    [IO.File]::WriteAllText($metadataFile, ($metadataText -join "`n"), [Text.UTF8Encoding]::new($false))
    & $Python (Join-Path $repo 'scripts\rust-notices.py') $metadataFile (Join-Path $stage 'ThirdParty') --repository $repo
    if ($LASTEXITCODE -ne 0) { throw 'Dependency notices are incomplete.' }
    # The path-bearing metadata stays out of the package and public output.
    Remove-Item -LiteralPath $metadataFile
    Copy-Item -LiteralPath $exe -Destination (Join-Path $stage 'DiskBurrow.exe')
    Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination (Join-Path $stage 'LICENSE.txt')
    Copy-Item -LiteralPath (Join-Path $repo 'scripts\GETTING-STARTED-RUST.txt') -Destination (Join-Path $stage 'GETTING-STARTED.txt')
    $commit = (& git -C $repo rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not record source revision.' }
    $dirty = [bool](@(& git -C $repo status --porcelain).Count)
    $manifest = [ordered]@{ product='DiskBurrow'; version=$version; architecture='windows-x64';
        source_commit=$commit; source_dirty=$dirty; compiler='Rust 1.97.0 MSVC';
        executable_sha256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant();
        cargo_lock_sha256=(Get-FileHash -LiteralPath (Join-Path $repo 'rust\Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant();
        static_crt=$true; imports=$dependencies; status='Preview; manual parity checks pending';
        upstream_disktree='6c8d4ce6211bf135ff4e42890faebff1040e1846' }
    $manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $stage 'manifest.json') -Encoding UTF8
    foreach ($item in Get-ChildItem -LiteralPath $stage -Recurse -Force) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Reparse point in package.' }
        $relative = $item.FullName.Substring($stage.Length + 1)
        if ($relative -notmatch '^(DiskBurrow\.exe|LICENSE\.txt|GETTING-STARTED\.txt|manifest\.json|ThirdParty(\\.*)?)$') { throw 'Unexpected package file.' }
    }
    $archive = Join-Path $output "DiskBurrow-$version-rust-win-x64.zip"
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive -CompressionLevel Optimal
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText(($archive + '.sha256'), ($hash + '  ' + [IO.Path]::GetFileName($archive) + "`n"), [Text.UTF8Encoding]::new($false))
    Write-Output $archive
    Write-Output $hash
} finally {
    $env:RUSTFLAGS = $oldFlags
    $env:CARGO_TARGET_DIR = $oldTarget
}
