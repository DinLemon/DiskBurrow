# Hosted Windows runners are elevated. Test the unchanged ordinary-user product
# under a disposable standard account instead of adding an elevation bypass.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true' -or -not $env:GITHUB_WORKSPACE) {
    throw 'This account-creation fixture is only for a disposable GitHub Actions runner.'
}
$workspace = [IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$fixture = [IO.Path]::GetFullPath((Join-Path $workspace 'work\rust-proof'))
$extracted = [IO.Path]::GetFullPath((Join-Path $workspace 'work\rust-extracted'))
foreach ($path in @($fixture, $extracted)) {
    if (-not $path.StartsWith(($workspace.TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $path)) {
        throw 'Expected a new fixture path strictly inside the CI workspace.'
    }
}
$archives = @(Get-ChildItem -LiteralPath (Join-Path $workspace 'artifacts\rust-preview') -Filter '*.zip' -File)
if ($archives.Count -ne 1) { throw 'Expected one preview archive.' }
$archive = $archives[0]
$expected = (Get-Content -LiteralPath ($archive.FullName + '.sha256')).Split(' ')[0]
if ((Get-FileHash -LiteralPath $archive.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw 'Archive checksum mismatch.' }
New-Item -ItemType Directory -Path $fixture,$extracted,(Join-Path $fixture 'scan') | Out-Null
Expand-Archive -LiteralPath $archive.FullName -DestinationPath $extracted
$exe = Join-Path $extracted 'DiskBurrow.exe'
$manifest = Get-Content -LiteralPath (Join-Path $extracted 'manifest.json') -Raw | ConvertFrom-Json
if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -ne $manifest.executable_sha256) { throw 'Executable checksum mismatch.' }
if ((Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -cne $manifest.version -or
    $archive.Name -cne ('DiskBurrow-' + $manifest.version + '-rust-win-x64.zip')) {
    throw 'Archive, executable and manifest versions differ.'
}
$revision = (& git -C $workspace rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $manifest.source_commit -cne $revision -or $manifest.source_dirty -ne $false -or -not $manifest.static_crt) {
    throw 'Expected an unmodified static-CRT archive built from the checked-out revision.'
}
'DiskBurrow disposable fixture' | Set-Content -LiteralPath (Join-Path $fixture 'scan\small.txt')
[IO.File]::WriteAllBytes((Join-Path $fixture 'scan\large.bin'), [byte[]]::new(2097152))
$name = 'dbpreview-' + [guid]::NewGuid().ToString('N').Substring(0,8)
# This randomly generated password exists only in this process on the disposable
# runner. It is never printed, persisted, or supplied by the human user.
$password = ConvertTo-SecureString ([Convert]::ToBase64String([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)) + 'aA!2') -AsPlainText -Force
$created = $false
$process = $null
try {
    $account = New-LocalUser -Name $name -Password $password -Description 'Disposable DiskBurrow CI ordinary-user fixture' -AccountNeverExpires
    $created = $true
    $users = Get-LocalGroup -SID 'S-1-5-32-545'
    Add-LocalGroupMember -Group $users -Member $account
    $sid = '*' + $account.SID.Value
    & icacls.exe $extracted /grant ($sid + ':(OI)(CI)RX') /T /Q | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not grant fixture executable access.' }
    & icacls.exe $fixture /grant ($sid + ':(OI)(CI)M') /T /Q | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not grant disposable data access.' }
    Start-Service -Name seclogon
    $credential = [Management.Automation.PSCredential]::new(($env:COMPUTERNAME + '\' + $name), $password)
    $arguments = '--data-dir "' + $fixture + '\data" --scan-root "' + $fixture + '\scan" --verify-runtime "' + $fixture + '\runtime.json"'
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -WorkingDirectory $extracted -Credential $credential -LoadUserProfile -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(60000)) { $process.Kill(); throw 'Ordinary-user executable verification timed out.' }
    if ($process.ExitCode -ne 0) { throw 'Ordinary-user extracted executable verification failed.' }
    $proof = Get-Content -LiteralPath (Join-Path $fixture 'runtime.json') -Raw | ConvertFrom-Json
    if (-not $proof.ordinary_user -or -not $proof.traversal_completed -or $proof.largest_files -ne 2 -or $proof.map_tiles -lt 1 -or $proof.appearances.Count -ne 4) {
        throw 'Ordinary-user runtime proof incomplete.'
    }
    $scan = Join-Path $fixture 'scan'
    if ($proof.product_version -cne $manifest.version -or
        [IO.Path]::GetFullPath($proof.executable) -ine $exe -or
        [IO.Path]::GetFullPath($proof.scan_root).TrimEnd('\') -ine $scan.TrimEnd('\')) {
        throw 'Runtime proof does not identify the extracted executable and isolated scan fixture.'
    }
    $appearances = @($proof.appearances | ForEach-Object { $_.language + '/' + $_.theme } | Sort-Object -Unique)
    if (($appearances -join ',') -cne 'en/dark,en/light,ru/dark,ru/light') { throw 'Language and theme proof incomplete.' }
    $projections = @($proof.display_projections | ForEach-Object { [string]$_.depth + '/' + [string]$_.show_hidden } | Sort-Object -Unique)
    if (($projections -join ',') -cne '1/False,1/True,3/False,3/True,6/False,6/True' -or
        $proof.display_settings.theme -cne 'system' -or $proof.display_settings.ui_scale_percent -ne 150 -or
        @($proof.age_bands | Where-Object { $_ -lt 0 -or $_ -gt 5 }).Count -gt 0 -or $proof.selected_count -ne 1) {
        throw 'Display and selected-export proof incomplete.'
    }
    $selectedPath = Join-Path $fixture 'runtime.selected.txt'
    if (-not $proof.map_panel.explicit_launch_opened_map -or
        $proof.map_panel.marked_roots -ne 1 -or -not $proof.map_panel.forecast_worker_finished -or
        $proof.map_panel.review_created -or [string]::IsNullOrWhiteSpace($proof.map_panel.forecast)) {
        throw 'Map-panel launch, marked selection or read-only forecast proof incomplete.'
    }
    $selected = [IO.File]::ReadAllText($selectedPath, [Text.UTF8Encoding]::new($false, $true))
    if ([string]::IsNullOrWhiteSpace($selected) -or $selected -notmatch 'large\.bin') {
        throw 'Selected UTF-8 export does not contain the indexed fixture.'
    }
    $snapshotPath = Join-Path $fixture 'runtime.snapshot.json'
    if ([IO.Path]::GetFullPath($proof.snapshot_export) -ine $snapshotPath) { throw 'Unexpected snapshot export location.' }
    $snapshot = Get-Content -LiteralPath $snapshotPath -Raw | ConvertFrom-Json
    $observed = @($snapshot.LargestFiles | Sort-Object Path)
    $actual = @(Get-ChildItem -LiteralPath $scan -File | Sort-Object FullName)
    if (-not $snapshot.TraversalCompleted -or $observed.Count -ne 2 -or $snapshot.Issues.Count -ne 0) { throw 'Exported scan is incomplete.' }
    for ($i = 0; $i -lt $actual.Count; $i++) {
        if ([IO.Path]::GetFullPath($observed[$i].Path) -ine $actual[$i].FullName -or $observed[$i].LogicalBytes -ne $actual[$i].Length) {
            throw 'Exported file path or logical byte count does not match the real fixture.'
        }
    }
} finally {
    if ($null -ne $process -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit(10000) | Out-Null }
    if ($created) { Remove-LocalUser -Name $name }
    $password.Dispose()
}
