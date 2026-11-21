# Builds external libraries for a core library and its transitive dependencies, then runs `mote test`.
# Usage: test-core.ps1 NAME [--core DIR] [-- FILTER]
$ErrorActionPreference = "Stop"
$root = (Resolve-Path "$PSScriptRoot\..").Path
$core = Join-Path $root "mote_std"
$name = ""
$filter = ""

for ($i = 0; $i -lt $args.Count; $i++) {
    $arg = [string]$args[$i]
    switch ($arg) {
        "--core" {
            $i++
            $core = (Resolve-Path ([string]$args[$i])).Path
        }
        "--" {
            $i++
            $filter = [string]$args[$i]
        }
        default {
            if ($arg.StartsWith("-")) { throw "unknown argument: $arg" }
            $name = $arg
        }
    }
}

if (-not $name) { throw "usage: test-core.ps1 NAME [--core DIR] [-- FILTER]" }
if (-not (Test-Path (Join-Path $core "$name\mote.toml"))) { throw "no library $name in $core" }

$seen = [System.Collections.Generic.HashSet[string]]::new()
$queue = [System.Collections.Generic.Queue[string]]::new()
$null = $seen.Add($name)
$queue.Enqueue($name)

while ($queue.Count -gt 0) {
    $current = $queue.Dequeue()
    foreach ($sub in @("src", "tests")) {
        $dir = Join-Path $core "$current\$sub"
        if (-not (Test-Path $dir)) { continue }
        $matches = Get-ChildItem -Path $dir -Filter "*.mote" -Recurse | Select-String -Pattern 'from\s+([a-z_][a-z0-9_]*)' -AllMatches
        foreach ($m in $matches) {
            foreach ($g in $m.Matches) {
                $used = $g.Groups[1].Value
                if (-not $seen.Contains($used) -and (Test-Path (Join-Path $core "$used\mote.toml"))) {
                    $null = $seen.Add($used)
                    $queue.Enqueue($used)
                }
            }
        }
    }
}

$global:LASTEXITCODE = 0
foreach ($lib in $seen) {
    & "$root\tools\build-std-libs.ps1" --core $core $lib
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

$global:LASTEXITCODE = 0
$moteExe = if ($env:MOTE) { $env:MOTE } else { "mote" }
$pkgDir = Join-Path $core $name
if ($filter) {
    & $moteExe test --core $core --filter $filter $pkgDir
} else {
    & $moteExe test --core $core $pkgDir
}
exit $LASTEXITCODE
