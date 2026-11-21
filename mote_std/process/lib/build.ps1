$ErrorActionPreference = "Stop"
& "$PSScriptRoot\..\..\..\tools\win\build-zig-lib.ps1" -Name "process" -Source "$PSScriptRoot\process.zig" -OutDir $env:MOTE_LIB_OUT
