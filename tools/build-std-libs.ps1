# Builds the external library of each core library into lib/x86_64-windows/ of its package (or --out DIR/<name>/).
# Usage: build-std-libs.ps1 [--check] [--core DIR] [--out DIR] [name]
$ErrorActionPreference = "Stop"
$root = (Resolve-Path "$PSScriptRoot\..").Path
$core = Join-Path $root "mote_std"
$check = $false
$only = ""
$outroot = ""

for ($i = 0; $i -lt $args.Count; $i++) {
    $arg = [string]$args[$i]
    switch ($arg.ToLowerInvariant()) {
        "--check" { $check = $true }
        "-check"  { $check = $true }
        "--core"  {
            if ($i + 1 -ge $args.Count) { throw "--core needs a directory" }
            $i++
            $core = (Resolve-Path ([string]$args[$i])).Path
        }
        "-core"   {
            if ($i + 1 -ge $args.Count) { throw "-Core needs a directory" }
            $i++
            $core = (Resolve-Path ([string]$args[$i])).Path
        }
        "--out"   {
            if ($i + 1 -ge $args.Count) { throw "--out needs a directory" }
            $i++
            $d = [string]$args[$i]
            New-Item -ItemType Directory -Force -Path $d | Out-Null
            $outroot = (Resolve-Path $d).Path
        }
        "-out"    {
            if ($i + 1 -ge $args.Count) { throw "-Out needs a directory" }
            $i++
            $d = [string]$args[$i]
            New-Item -ItemType Directory -Force -Path $d | Out-Null
            $outroot = (Resolve-Path $d).Path
        }
        default {
            if ($arg.StartsWith("-")) { throw "unknown argument: $arg" }
            $only = $arg
        }
    }
}

$triple = "x86_64-windows"

function Test-DeclaresExtern([string]$dir) {
    $src = Join-Path $dir "src"
    if (-not (Test-Path $src)) { return $false }
    $hit = Get-ChildItem -Path $src -Filter "*.mote" -Recurse | Select-String -Pattern '^\s*(pub\s+)?extern\s+"' | Select-Object -First 1
    return ($null -ne $hit)
}

function Test-HasBuilder([string]$dir) {
    return (Test-Path (Join-Path $dir "lib\build.ps1")) -or (Test-Path (Join-Path $dir "rust\Cargo.toml"))
}

function Build-Library([string]$dir) {
    $name = Split-Path $dir -Leaf
    $out = Join-Path $dir "lib\$triple"
    $target = Join-Path $dir "rust\target"
    if ($outroot) {
        $out = Join-Path $outroot $name
        $target = Join-Path $out "rust-target"
    }
    New-Item -ItemType Directory -Force -Path $out | Out-Null

    $psBuilder = Join-Path $dir "lib\build.ps1"
    if (Test-Path $psBuilder) {
        $prevOut = $env:MOTE_LIB_OUT
        $prevHdr = $env:MOTE_HEADER_DIR
        $prevNetTarget = $env:MOTE_NET_RUST_TARGET
        $env:MOTE_LIB_OUT = $out
        $env:MOTE_HEADER_DIR = Join-Path $root "abi"
        if ($outroot) {
            $env:MOTE_NET_RUST_TARGET = Join-Path $out "rust-target"
        }
        Push-Location $dir
        try {
            & $psBuilder
            if ($LASTEXITCODE -ne 0) { throw "build.ps1 failed for $name" }
        } finally {
            Pop-Location
            $env:MOTE_LIB_OUT = $prevOut
            $env:MOTE_HEADER_DIR = $prevHdr
            $env:MOTE_NET_RUST_TARGET = $prevNetTarget
        }
    } else {
        $manifest = Join-Path $dir "rust\Cargo.toml"
        & cargo build --release --manifest-path $manifest --target-dir $target
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed for $name" }
        $dllSrc = Join-Path $target "release\$name.dll"
        $tmpDll = Join-Path $out ".$name.dll.$PID"
        $destDll = Join-Path $out "$name.dll"
        Copy-Item -Force -Path $dllSrc -Destination $tmpDll
        Move-Item -Force -Path $tmpDll -Destination $destDll
    }

    $expectedDll = Join-Path $out "$name.dll"
    if (-not (Test-Path $expectedDll)) {
        throw "${name}: the build wrote no $expectedDll"
    }
    Write-Output "built $expectedDll"
}

if ($only -and -not (Test-Path (Join-Path $core "$only\mote.toml"))) {
    throw "no library $only in $core"
}

$failed = 0
$builtLibs = 0
$libraries = 0

foreach ($dirObj in (Get-ChildItem -Path $core -Directory | Sort-Object Name)) {
    $dir = $dirObj.FullName
    if (-not (Test-Path (Join-Path $dir "mote.toml"))) { continue }
    $name = $dirObj.Name
    if ($name -eq "std") { continue }
    if ($only -and ($name -ne $only)) { continue }
    $libraries++
    if (-not (Test-DeclaresExtern $dir)) { continue }
    $builtLibs++
    if (-not (Test-HasBuilder $dir)) {
        Write-Error "${name}: declares an external library and has no lib\build.ps1 or rust\Cargo.toml"
        $failed++
    } elseif (-not $check) {
        try {
            Build-Library $dir
        } catch {
            Write-Error $_
            $failed++
        }
    }
}

if ($failed -ne 0) { exit 1 }
if ($check) {
    Write-Output "ok: $libraries libraries, $builtLibs with a shared library"
}
