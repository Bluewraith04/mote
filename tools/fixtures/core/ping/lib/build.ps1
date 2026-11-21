# Builds ping.dll and ping.lib into $env:MOTE_LIB_OUT; $env:MOTE_HEADER_DIR holds mote.h.
$ErrorActionPreference = "Stop"

$out = $env:MOTE_LIB_OUT
$hdr = $env:MOTE_HEADER_DIR
New-Item -ItemType Directory -Force -Path $out | Out-Null

function Get-NewestSubdir([string]$Parent) {
    if (-not (Test-Path $Parent)) { return $null }
    Get-ChildItem -Path $Parent -Directory | Sort-Object Name | Select-Object -Last 1 -ExpandProperty FullName
}

$vcTools = $env:VCToolsInstallDir
if ($vcTools -and (Test-Path (Join-Path $vcTools "bin\Hostx64\x64\cl.exe"))) {
    $binDir = Join-Path $vcTools "bin\Hostx64\x64"
} else {
    $pf86 = if (${env:ProgramFiles(x86)}) { ${env:ProgramFiles(x86)} } else { "C:\Program Files (x86)" }
    $vswhere = Join-Path $pf86 "Microsoft Visual Studio\Installer\vswhere.exe"
    $vsRoot = (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1).Trim()
    $vcVer = Get-NewestSubdir (Join-Path $vsRoot "VC\Tools\MSVC")
    $binDir = Join-Path $vcVer "bin\Hostx64\x64"
    $incDirs = @((Join-Path $vcVer "include"))
    $libDirs = @((Join-Path $vcVer "lib\x64"))
    $sdkInc = Get-NewestSubdir (Join-Path $pf86 "Windows Kits\10\Include")
    if ($sdkInc) {
        $incDirs += (Join-Path $sdkInc "ucrt"), (Join-Path $sdkInc "um"), (Join-Path $sdkInc "shared")
    }
    $sdkLib = Get-NewestSubdir (Join-Path $pf86 "Windows Kits\10\Lib")
    if ($sdkLib) {
        $libDirs += (Join-Path $sdkLib "um\x64"), (Join-Path $sdkLib "ucrt\x64")
    }
    $env:INCLUDE = ($incDirs -join ";") + $(if ($env:INCLUDE) { ";" + $env:INCLUDE } else { "" })
    $env:LIB = ($libDirs -join ";") + $(if ($env:LIB) { ";" + $env:LIB } else { "" })
}

$cl = Join-Path $binDir "cl.exe"
$libExe = Join-Path $binDir "lib.exe"
$env:PATH = "$binDir;$env:PATH"

$src = Join-Path $PSScriptRoot "ping.c"
$dllOut = Join-Path $out "ping.dll"
$libOut = Join-Path $out "ping.lib"
$dllObj = Join-Path $out "ping_dll.obj"
$impLib = Join-Path $out "ping_imp.lib"
$expFile = Join-Path $out "ping_imp.exp"
$statObj = Join-Path $out "ping.obj"

& $cl /nologo /O2 /MD /LD /DPING_DLL "/I$hdr" "/Fo$dllObj" "/Fe$dllOut" $src /link "/IMPLIB:$impLib"
if ($LASTEXITCODE -ne 0) { throw "cl.exe DLL build failed for ping" }
Remove-Item -Force -ErrorAction SilentlyContinue $dllObj, $impLib, $expFile

& $cl /nologo /O2 /MD /c "/I$hdr" "/Fo$statObj" $src
if ($LASTEXITCODE -ne 0) { throw "cl.exe object build failed for ping" }
& $libExe /NOLOGO "/OUT:$libOut" $statObj
if ($LASTEXITCODE -ne 0) { throw "lib.exe archive build failed for ping" }
Remove-Item -Force -ErrorAction SilentlyContinue $statObj
