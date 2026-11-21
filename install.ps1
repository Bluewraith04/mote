# Installs the `mote` CLI via `cargo install` and copies `mote_std\` to `$env:MOTE_HOME\mote_std` (default `~/.mote/mote_std`).
# Usage: .\install.ps1 [-CompileStd]
param(
    [switch]$CompileStd
)
$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
$moteHome = if ($env:MOTE_HOME) { $env:MOTE_HOME } else { Join-Path $env:USERPROFILE ".mote" }

if ($CompileStd) {
    & "$root\compile-std.ps1"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

& cargo install --path (Join-Path $root "mote") --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$dest = Join-Path $moteHome "mote_std"
$stage = Join-Path $moteHome ".mote_std.$PID"
New-Item -ItemType Directory -Force -Path $moteHome | Out-Null
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
& robocopy (Join-Path $root "mote_std") $stage /E /XD target rust\target /NFL /NDL /NJH /NJS /NC /NS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "robocopy failed with exit code $LASTEXITCODE" }
$global:LASTEXITCODE = 0
if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
Move-Item -Force $stage $dest
Write-Output "installed mote_std -> $dest"
