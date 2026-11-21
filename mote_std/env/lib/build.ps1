$ErrorActionPreference = "Stop"
& "$PSScriptRoot\..\..\..\tools\win\build-zig-lib.ps1" -Name "env" -Source "$PSScriptRoot\env.zig" -OutDir $env:MOTE_LIB_OUT
