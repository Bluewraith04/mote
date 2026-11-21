$ErrorActionPreference = "Stop"
& "$PSScriptRoot\..\..\..\tools\win\build-zig-lib.ps1" -Name "term" -Source "$PSScriptRoot\term.zig" -OutDir $env:MOTE_LIB_OUT
