$ErrorActionPreference = "Stop"
& "$PSScriptRoot\..\..\..\tools\win\build-zig-lib.ps1" -Name "fs" -Source "$PSScriptRoot\fs.zig" -OutDir $env:MOTE_LIB_OUT
