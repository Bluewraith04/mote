$ErrorActionPreference = "Stop"
& "$PSScriptRoot\..\..\..\tools\win\build-zig-lib.ps1" -Name "net" -Source "$PSScriptRoot\net.zig" -OutDir $env:MOTE_LIB_OUT
$manifest = (Resolve-Path "$PSScriptRoot\..\rust\Cargo.toml").Path
$target = if ($env:MOTE_NET_RUST_TARGET) { $env:MOTE_NET_RUST_TARGET } else { (Resolve-Path "$PSScriptRoot\..\rust").Path + "\target" }
& cargo build --release --manifest-path $manifest --target-dir $target
if ($LASTEXITCODE -ne 0) { throw "cargo build failed for nettls" }
$dllSrc = Join-Path $target "release\nettls.dll"
$tmpDll = Join-Path $env:MOTE_LIB_OUT ".nettls.dll.$PID"
$outDll = Join-Path $env:MOTE_LIB_OUT "nettls.dll"
Copy-Item -Force -Path $dllSrc -Destination $tmpDll
Move-Item -Force -Path $tmpDll -Destination $outDll
