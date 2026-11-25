# Usage: powershell -File test-install.ps1 -Bin <path-to-built-tally.exe>
# Runs install.ps1 against a fake release directory: install, install again (no-op), corrupt checksum (installs nothing),
# user PATH holds the install dir once, uninstall (binary removed, PATH entry removed, data kept).
# The user PATH is restored to its original value at the end whatever happens.
param([Parameter(Mandatory)][string]$Bin)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$ver = ((& $Bin --version) -split ' ')[1]
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
$name = "tally-$ver-$arch-pc-windows-msvc.zip"
$t = Join-Path ([IO.Path]::GetTempPath()) ("tally-itest-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path "$t\rel", "$t\stage", "$t\home" | Out-Null
$origPath = [Environment]::GetEnvironmentVariable('Path', 'User')
function Fail($m) { throw "FAIL: $m" }
try {
    Copy-Item $Bin "$t\stage\tally.exe"
    Compress-Archive -Path "$t\stage\tally.exe" -DestinationPath "$t\rel\$name"
    $h = (Get-FileHash -Algorithm SHA256 "$t\rel\$name").Hash.ToLower()
    [IO.File]::WriteAllText("$t\rel\SHA256SUMS", "$h  $name`n")
    $env:TALLY_RELEASE_BASE = ([Uri]"$t\rel").AbsoluteUri
    $env:TALLY_VERSION = $ver
    $env:TALLY_HOME = "$t\home"
    $dir = "$t\bin"
    $count = { ([Environment]::GetEnvironmentVariable('Path', 'User') -split ';' | Where-Object { $_.TrimEnd('\') -ieq $dir }).Count }

    # (a) first run installs a working binary
    $env:TALLY_INSTALL_DIR = $dir
    & powershell -NoProfile -File "$here\install.ps1" | Out-Null
    if ($LASTEXITCODE) { Fail 'first install exited non-zero' }
    if ((& "$dir\tally.exe" --version) -ne "tally $ver") { Fail "installed binary does not report tally $ver" }

    # (b) second run is a no-op, binary untouched, PATH has the dir exactly once (e)
    $m1 = (Get-Item "$dir\tally.exe").LastWriteTimeUtc
    Start-Sleep -Seconds 1
    $out = (& powershell -NoProfile -File "$here\install.ps1") -join "`n"
    if ($out -notmatch 'already installed') { Fail "second install did not say 'already installed': $out" }
    if ((Get-Item "$dir\tally.exe").LastWriteTimeUtc -ne $m1) { Fail 'binary changed on the second run' }
    if ((& $count) -ne 1) { Fail "user PATH holds the install dir $(& $count) times, expected 1" }

    # (c) a corrupt checksum installs nothing
    [IO.File]::WriteAllText("$t\rel\SHA256SUMS", ('0' * 64) + "  $name`n")
    $env:TALLY_INSTALL_DIR = "$t\clean"
    $ErrorActionPreference = 'Continue'   # 5.1 turns a native command's stderr into a terminating error under 'Stop'
    $null = & powershell -NoProfile -File "$here\install.ps1" 2>&1
    $code = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($code -eq 0) { Fail 'corrupt checksum was accepted' }
    if (Test-Path "$t\clean\tally.exe") { Fail 'binary installed despite a bad checksum' }

    # (d) uninstall removes the binary and the PATH entry, keeps the data
    'data' | Set-Content "$t\home\tally.db"
    $env:TALLY_INSTALL_DIR = $dir
    & powershell -NoProfile -File "$here\install.ps1" -Uninstall | Out-Null
    if ($LASTEXITCODE) { Fail 'uninstall exited non-zero' }
    if (Test-Path "$dir\tally.exe") { Fail 'uninstall left the binary' }
    if (-not (Test-Path "$t\home\tally.db")) { Fail 'uninstall touched the data directory' }
    if ((& $count) -ne 0) { Fail 'uninstall left the PATH entry' }
    Write-Host 'install.ps1 ok'
} finally {
    [Environment]::SetEnvironmentVariable('Path', $origPath, 'User')
    Remove-Item $t -Recurse -Force -ErrorAction SilentlyContinue
}
