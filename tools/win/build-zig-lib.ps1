# Shared helper to build a Zig shared library as <Name>.dll into $OutDir.
param(
    [Parameter(Mandatory = $true)][string]$Name,
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$OutDir
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$outDll = Join-Path $OutDir "$Name.dll"
if (Test-Path $outDll) {
    $outTime = (Get-Item $outDll).LastWriteTimeUtc
    if ($outTime -ge (Get-Item $Source).LastWriteTimeUtc) {
        exit 0
    }
}
$tmpDll = Join-Path $OutDir ".$Name.$PID.dll"

$zig = Get-Command zig -ErrorAction SilentlyContinue
if ($zig) {
    & $zig.Source build-lib $Source -target x86_64-windows-gnu -dynamic -OReleaseSafe -fstrip -fno-stack-check "-femit-bin=$tmpDll" -fno-emit-h -fno-emit-implib
    if ($LASTEXITCODE -ne 0) { throw "zig build-lib failed for $Name" }
} else {
    $wsl = Get-Command wsl.exe -ErrorAction SilentlyContinue
    if (-not $wsl) { throw "Neither zig nor wsl.exe found to build $Name.zig" }
    $srcAbs = ((Resolve-Path $Source).Path) -replace '\\', '/'
    $tmpAbs = ([System.IO.Path]::GetFullPath($tmpDll)) -replace '\\', '/'
    $srcWsl = [string]($null | & $wsl.Source --exec wslpath -a -u $srcAbs)
    if ($LASTEXITCODE -ne 0 -or -not $srcWsl) { throw "wslpath failed for $srcAbs" }
    $outWsl = [string]($null | & $wsl.Source --exec wslpath -a -u $tmpAbs)
    if ($LASTEXITCODE -ne 0 -or -not $outWsl) { throw "wslpath failed for $tmpAbs" }
    $srcWsl = $srcWsl.Trim()
    $outWsl = $outWsl.Trim()
    $zigCmd = "set -e; export PATH=`$HOME/.zvm/bin:`$PATH; cd /tmp; out=/tmp/mote-$Name-`$`$.dll; zig build-lib '$srcWsl' -target x86_64-windows-gnu -dynamic -OReleaseSafe -fstrip -fno-stack-check --cache-dir /tmp/mote-zig-cache -femit-bin=`$out -fno-emit-h -fno-emit-implib; cp -f `$out '$outWsl'; rm -f `$out"
    $null | & $wsl.Source --exec bash -lc $zigCmd
    if ($LASTEXITCODE -ne 0) { throw "wsl zig build-lib failed for $Name" }
}

Move-Item -Force -Path $tmpDll -Destination $outDll
