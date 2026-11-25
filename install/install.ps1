# Tally installer for Windows (PowerShell 5.1+). Installs the binary only: no service, no tracking.
# Consent and the explainer come with `tally setup`.
#
#   irm https://github.com/sectersion/tally/releases/latest/download/install.ps1 | iex
#   .\install.ps1 -Uninstall        # removes the binary and its PATH entry only; your data stays
#
# Environment: TALLY_VERSION (default: latest), TALLY_INSTALL_DIR (default: %LOCALAPPDATA%\Programs\Tally),
#   TALLY_RELEASE_BASE (flat directory or URL holding the archive and SHA256SUMS; needs TALLY_VERSION; for CI and offline installs).
param([switch]$Uninstall)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repo = 'sectersion/tally'
$dir = if ($env:TALLY_INSTALL_DIR) { $env:TALLY_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\Tally' }
$exe = Join-Path $dir 'tally.exe'

function Get-UserPathEntries {
    $p = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($p) { return @($p.Split(';') | Where-Object { $_ }) }
    return @()
}
function Test-SameDir($a, $b) { return $a.TrimEnd('\') -ieq $b.TrimEnd('\') }

if ($Uninstall) {
    $home_dir = if ($env:TALLY_HOME) { $env:TALLY_HOME } else { Join-Path $env:LOCALAPPDATA 'tally' }
    Write-Host "data kept in $home_dir; to delete it run: tally service uninstall --purge"
    Write-Host '(run that before removing the binary if you want it gone)'
    if (Test-Path $exe) { Remove-Item $exe -Force }
    $entries = @(Get-UserPathEntries | Where-Object { -not (Test-SameDir $_ $dir) })
    [Environment]::SetEnvironmentVariable('Path', ($entries -join ';'), 'User')
    Write-Host "removed $exe"
    return
}

switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { $arch = 'x86_64' }
    'ARM64' { $arch = 'aarch64' }
    default { throw "unsupported architecture $($env:PROCESSOR_ARCHITECTURE)" }
}
$target = "$arch-pc-windows-msvc"

if ($env:TALLY_RELEASE_BASE) {
    if (-not $env:TALLY_VERSION) { throw 'TALLY_RELEASE_BASE needs TALLY_VERSION' }
    $ver = $env:TALLY_VERSION
    $base = $env:TALLY_RELEASE_BASE.TrimEnd('/')
} else {
    $ver = $env:TALLY_VERSION
    if (-not $ver) {
        $req = [Net.WebRequest]::Create("https://github.com/$repo/releases/latest")
        $req.AllowAutoRedirect = $false
        $resp = $req.GetResponse()
        $loc = $resp.Headers['Location']
        $resp.Close()
        $ver = ($loc -split '/v')[-1]
    }
    $base = "https://github.com/$repo/releases/download/v$ver"
}

if (Test-Path $exe) {
    $have = (& $exe --version 2>$null)
    if ($have -eq "tally $ver") { Write-Host "tally $ver already installed in $dir"; return }
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("tally-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS')
    # the archive is `-unsigned` when the release was built without signing secrets
    $line = Get-Content (Join-Path $tmp 'SHA256SUMS') | Where-Object { $_ -match "  tally-$([regex]::Escape($ver))-$target(-unsigned)?\.zip$" } | Select-Object -First 1
    if (-not $line) { throw "no archive for $target in SHA256SUMS (version $ver)" }
    $want = ($line -split '\s+')[0]
    $name = ($line -split '\s+')[-1]
    $zip = Join-Path $tmp $name
    Invoke-WebRequest -UseBasicParsing "$base/$name" -OutFile $zip
    $got = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
    if ($got -ne $want.ToLower()) { throw "checksum mismatch for $name (expected $want, got $got); nothing installed" }

    Expand-Archive -Path $zip -DestinationPath (Join-Path $tmp 'x') -Force
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $wasRunning = $false
    if (Test-Path $exe) {
        & $exe service status --quiet *> $null
        if ($LASTEXITCODE -eq 0) { $wasRunning = $true; & $exe service stop *> $null }
    }
    Copy-Item (Join-Path $tmp 'x\tally.exe') $exe -Force
    if ($wasRunning) { & $exe service start *> $null }
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

$entries = Get-UserPathEntries
if (-not ($entries | Where-Object { Test-SameDir $_ $dir })) {
    [Environment]::SetEnvironmentVariable('Path', (($entries + $dir) -join ';'), 'User')
    Write-Host "added $dir to your user PATH (open a new terminal to use it)"
}
Write-Host "installed tally $ver to $dir"
Write-Host 'Next: run "tally setup", then "tally service install" to start it at every login'
