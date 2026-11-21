# Builds pane.dll into $env:MOTE_LIB_OUT using MSVC cl.exe and link.exe.
$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
$outDir = $env:MOTE_LIB_OUT
if (-not $outDir) { throw "MOTE_LIB_OUT is not set" }
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$outDll = Join-Path $outDir "pane.dll"
$sourcesToCheck = @(
    "$here\pane_fonts.c", "$here\pane_fonts.h", "$here\pane_layout.c",
    "$here\pane_win32.c", "$here\pane_win32.h", "$here\pane_window.c"
)
if (Test-Path $outDll) {
    $outTime = (Get-Item $outDll).LastWriteTimeUtc
    $newer = $sourcesToCheck | Where-Object { (Get-Item $_).LastWriteTimeUtc -gt $outTime }
    if (-not $newer) { exit 0 }
}

if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) { throw "cl.exe not in PATH and vswhere.exe not found" }
    $vsPath = (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1).Trim()
    if (-not $vsPath) { throw "Visual Studio C++ Build Tools installation not found" }
    $vcvars = Join-Path $vsPath "VC\Auxiliary\Build\vcvars64.bat"
    if (-not (Test-Path $vcvars)) { throw "vcvars64.bat not found at $vcvars" }
    $envLines = cmd /c "`"$vcvars`" >nul 2>&1 && set"
    foreach ($line in $envLines) {
        if ($line -match '^([^=]+)=(.*)$') {
            [Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
        }
    }
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("mote-pane-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $work | Out-Null

try {
    # Generate C byte arrays for the 4 embedded TTF fonts (MSVC lacks GNU .incbin).
    $fontMap = @(
        @{ Symbol = "font_sans"; File = "$here\vendor\font\DejaVuSans.ttf" },
        @{ Symbol = "font_bold"; File = "$here\vendor\font\DejaVuSans-Bold.ttf" },
        @{ Symbol = "font_serif"; File = "$here\vendor\font\DejaVuSerif.ttf" },
        @{ Symbol = "font_mono"; File = "$here\vendor\font\DejaVuSansMono.ttf" }
    )
    $fontC = Join-Path $work "pane_fonts_win.c"
    $sw = [System.IO.StreamWriter]::new($fontC, $false, [System.Text.Encoding]::ASCII)
    try {
        foreach ($entry in $fontMap) {
            $bytes = [System.IO.File]::ReadAllBytes($entry.File)
            $sw.WriteLine("const unsigned int $($entry.Symbol)_size = $($bytes.Length);")
            $sw.WriteLine("const unsigned char $($entry.Symbol)_data[] = {")
            $sb = [System.Text.StringBuilder]::new(8192)
            for ($i = 0; $i -lt $bytes.Length; $i++) {
                $null = $sb.Append($bytes[$i].ToString())
                if ($i + 1 -lt $bytes.Length) { $null = $sb.Append(",") }
                if (($i % 24) -eq 23) {
                    $sw.WriteLine($sb.ToString())
                    $null = $sb.Clear()
                }
            }
            if ($sb.Length -gt 0) { $sw.WriteLine($sb.ToString()) }
            $sw.WriteLine("};")
        }
    } finally {
        $sw.Dispose()
    }

    $raylib = Join-Path $here "vendor\raylib\src"
    $objs = [System.Collections.Generic.List[string]]::new()

    function Compile-Obj([string]$src, [string[]]$extraFlags) {
        $obj = Join-Path $work ([System.IO.Path]::GetFileNameWithoutExtension($src) + ".obj")
        $args = @("/nologo", "/c", "/O2", "/MD", "/TC", "/std:c11", "/I$here", "/Fo$obj") + $extraFlags + @($src)
        & cl.exe @args
        if ($LASTEXITCODE -ne 0) { throw "cl.exe failed compiling $src" }
        $objs.Add($obj)
    }

    Compile-Obj $fontC @("/W0")
    foreach ($f in @("pane_fonts.c", "pane_layout.c", "pane_win32.c", "pane_window.c")) {
        Compile-Obj (Join-Path $here $f) @("/W2", "/D_CRT_SECURE_NO_WARNINGS")
    }
    $rlFlags = @("/W0", "/DPLATFORM_DESKTOP", "/DGRAPHICS_API_OPENGL_33", "/DSUPPORT_MODULE_RMODELS=0", "/DSUPPORT_MODULE_RAUDIO=0", "/D_CRT_SECURE_NO_WARNINGS", "/I$raylib", "/I$raylib\external\glfw\include")
    foreach ($f in @("rcore.c", "rshapes.c", "rtext.c", "rtextures.c")) {
        Compile-Obj (Join-Path $raylib $f) $rlFlags
    }
    Compile-Obj (Join-Path $raylib "rglfw.c") @("/W0", "/D_GLFW_WIN32", "/D_CRT_SECURE_NO_WARNINGS", "/I$raylib", "/I$raylib\external\glfw\include")

    $tmpDll = Join-Path $outDir ".pane.dll.$PID"
    $tmpLib = Join-Path $work "pane.lib"
    $tmpExp = Join-Path $work "pane.exp"
    $linkArgs = @("/NOLOGO", "/DLL", "/OUT:$tmpDll", "/IMPLIB:$tmpLib") + $objs.ToArray() + @(
        "user32.lib", "gdi32.lib", "winmm.lib", "shell32.lib", "opengl32.lib", "comdlg32.lib"
    )
    & link.exe @linkArgs
    if ($LASTEXITCODE -ne 0) { throw "link.exe failed linking pane.dll" }
    Move-Item -Force -Path $tmpDll -Destination $outDll
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $work
}
