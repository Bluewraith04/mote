# Recompiles C/Rust libraries in mote_std\ (all, or the named library [name]),
# automatically selecting zig (lib\build.ps1) or rust (rust\Cargo.toml).
# Usage: .\compile-std.ps1 [--check] [name]
$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
& "$root\tools\build-std-libs.ps1" --core (Join-Path $root "mote_std") @args
exit $LASTEXITCODE
