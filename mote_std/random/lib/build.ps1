$ErrorActionPreference = "Stop"
& "$PSScriptRoot\..\..\..\tools\win\build-zig-lib.ps1" -Name "random" -Source "$PSScriptRoot\random.zig" -OutDir $env:MOTE_LIB_OUT
