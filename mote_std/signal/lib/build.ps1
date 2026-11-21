$ErrorActionPreference = "Stop"
& "$PSScriptRoot\..\..\..\tools\win\build-zig-lib.ps1" -Name "signal" -Source "$PSScriptRoot\signal.zig" -OutDir $env:MOTE_LIB_OUT
