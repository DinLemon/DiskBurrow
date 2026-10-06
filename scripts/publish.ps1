[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$')][string]$Version,
    [Parameter(Mandatory)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$outputProvider = $null
$outputDrive = $null
$outputPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory, [ref]$outputProvider, [ref]$outputDrive)
if ($outputProvider.Name -ne 'FileSystem') { throw 'Output directory must use the FileSystem provider.' }
$output = [IO.Path]::TrimEndingDirectorySeparator([IO.Path]::GetFullPath($outputPath))
if (Test-Path -LiteralPath $output) { throw 'Output directory must be new; existing directories are never removed or reused.' }
if ($output -eq [IO.Path]::GetPathRoot($output)) { throw 'A volume root is not an output directory.' }
# Check the resolved ancestor chain before creating anything. There are no recursive deletions.
for ($ancestor = Split-Path -Parent $output; $ancestor; $ancestor = Split-Path -Parent $ancestor) {
    if ((Test-Path -LiteralPath $ancestor) -and ((Get-Item -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Output ancestors must not be reparse points.' }
}
$parent = Split-Path -Parent $output
[IO.Directory]::CreateDirectory($parent) | Out-Null
New-Item -ItemType Directory -Path $output -ErrorAction Stop | Out-Null
'DiskBurrow package staging; created by publish.ps1' | Set-Content -LiteralPath (Join-Path $output '.diskburrow-package-output')
$publish = Join-Path $output 'published'
$manifestPath = Join-Path $output 'resolved-publish-outputs.txt'
$project = Join-Path $repo 'src/DiskBurrow.App/DiskBurrow.App.csproj'
$properties = @('-p:SelfContained=true', '-p:RuntimeFrameworkVersion=10.0.12', '-p:DebugType=none', '-p:DebugSymbols=false', '-p:RepositoryUrl=https://github.com/DinLemon/DiskBurrow')
& dotnet restore $project -r win-x64 --locked-mode @properties
if ($LASTEXITCODE -ne 0) { throw "Locked portable restore failed ($LASTEXITCODE)." }
& dotnet publish $project -c Release -r win-x64 --self-contained true --no-restore -o $publish @properties "-p:Version=$Version" "-p:PortableManifestPath=$manifestPath"
if ($LASTEXITCODE -ne 0) { throw "Portable publish failed ($LASTEXITCODE)." }
$files = @(Get-Content -LiteralPath $manifestPath | ForEach-Object { $_.Replace('\', '/').Trim() } | Where-Object { $_ -and -not $_.EndsWith('.pdb', [StringComparison]::OrdinalIgnoreCase) } | Sort-Object -Unique)
if ($files.Count -lt 10) { throw 'Publish manifest is incomplete.' }
foreach ($relative in $files) {
    if ([IO.Path]::IsPathFullyQualified($relative) -or $relative.Split('/') -contains '..' -or $relative -match '(^|/)(work|tests|obj|bin|\.git|\.superpowers)(/|$)' -or $relative -match '(?i)(\.db(-shm|-wal|-journal)?$|scan-report|\.log$)') { throw 'Publish manifest contains a disallowed entry.' }
    $source = [IO.Path]::GetFullPath((Join-Path $publish $relative))
    if (-not $source.StartsWith($publish + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or -not (Test-Path -LiteralPath $source -PathType Leaf) -or ((Get-Item -LiteralPath $source).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Publish output escaped its staging directory or is absent.' }
}
foreach ($required in @('DiskBurrow.exe','DiskBurrow.dll','coreclr.dll','hostpolicy.dll','e_sqlite3.dll','PresentationFramework.dll','DiskBurrow.runtimeconfig.json')) {
    if ($required -notin $files) { throw "Required publish output missing: $required" }
}
$runtime = Get-Content -LiteralPath (Join-Path $publish 'DiskBurrow.runtimeconfig.json') -Raw | ConvertFrom-Json
foreach ($framework in @('Microsoft.NETCore.App','Microsoft.WindowsDesktop.App')) {
    $included = @($runtime.runtimeOptions.includedFrameworks | Where-Object name -EQ $framework)
    if ($included.Count -ne 1 -or $included[0].version -ne '10.0.12') { throw "Bundled framework version mismatch: $framework" }
}
$supplemental = @('LICENSE', 'GETTING-STARTED.txt', 'PACKAGE-MANIFEST.json')
Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination (Join-Path $publish 'LICENSE')
Copy-Item -LiteralPath (Join-Path $repo 'scripts/GETTING-STARTED.txt') -Destination (Join-Path $publish 'GETTING-STARTED.txt')
@{ version = $Version; runtimeVersion = '10.0.12'; rid = 'win-x64'; repositoryUrl = 'https://github.com/DinLemon/DiskBurrow'; publishedFiles = $files; supplementalFiles = $supplemental } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $publish 'PACKAGE-MANIFEST.json') -Encoding utf8NoBOM
$files | Set-Content -LiteralPath (Join-Path $output 'publish-outputs.txt') -Encoding utf8NoBOM
$archive = Join-Path $output "DiskBurrow-$Version-win-x64.zip"
$entries = @($files + $supplemental | Sort-Object -Unique)
# Inspect shipped bytes as well as names. PDBs are omitted; compiler paths are normalized with PathMap.
foreach ($relative in $entries) {
    $bytes = [IO.File]::ReadAllBytes((Join-Path $publish $relative))
    foreach ($text in @([Text.Encoding]::UTF8.GetString($bytes), [Text.Encoding]::Unicode.GetString($bytes), [Text.Encoding]::Unicode.GetString($bytes, 1, $bytes.Length - 1))) {
        if ($text.IndexOf($repo, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or $text -match '(?i)[A-Z]:[\\/]Users[\\/]|[A-Z]:[\\/]CodexWork[\\/]') { throw "Local path metadata found in $relative." }
    }
}
$zip = [IO.Compression.ZipFile]::Open($archive, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($relative in $entries) {
        $entry = $zip.CreateEntry($relative, [IO.Compression.CompressionLevel]::Optimal)
        $entry.LastWriteTime = [DateTimeOffset]::new(2000,1,1,0,0,0,[TimeSpan]::Zero)
        $input = [IO.File]::OpenRead((Join-Path $publish $relative))
        $destination = $entry.Open()
        try { $input.CopyTo($destination) } finally { $destination.Dispose(); $input.Dispose() }
    }
} finally { $zip.Dispose() }
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $([IO.Path]::GetFileName($archive))" | Set-Content -LiteralPath ($archive + '.sha256') -Encoding ascii
Write-Output "Created $([IO.Path]::GetFileName($archive)); $($entries.Count) allowlisted entries."
