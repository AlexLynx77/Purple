# Builds Purple and assembles a ready-to-run folder: dist\Purple
#
#   .\scripts\package.ps1              # build (release) then assemble
#   .\scripts\package.ps1 -SkipBuild   # only assemble from the last build
#
# Layout of the result (the three programs must stay together):
#   purple.exe       the resident program (engine + tray icon): the one to start
#   purple-ui.exe    the window, opened from the tray icon
#   purplectl.exe    command line (diagnostics)
#   patcher\         ltk_patcher_host.exe, ltk_patcher_dll.dll (see scripts\get-patcher.ps1)
param([switch]$SkipBuild)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

if (-not $SkipBuild) {
    # Windows PowerShell 5.1 turns cargo's normal progress output on stderr into errors under
    # "Stop": relax it for the build and rely on the exit code instead.
    $ErrorActionPreference = "Continue"
    cargo build --release 2>&1 | ForEach-Object { "$_" }
    $buildFailed = $LASTEXITCODE -ne 0
    $ErrorActionPreference = "Stop"
    if ($buildFailed) { throw "cargo build failed" }
}

$patcher = Join-Path $root "vendor\ltk-patcher"
foreach ($file in "ltk_patcher_host.exe", "ltk_patcher_dll.dll") {
    if (-not (Test-Path (Join-Path $patcher $file))) {
        throw "$file is missing from vendor\ltk-patcher: run .\scripts\get-patcher.ps1 first"
    }
}

$dist = Join-Path $root "dist\Purple"
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Force (Join-Path $dist "patcher") | Out-Null
foreach ($exe in "purple.exe", "purple-ui.exe", "purplectl.exe") {
    Copy-Item (Join-Path $root "target\release\$exe") $dist
}
Copy-Item (Join-Path $patcher "*") (Join-Path $dist "patcher") -Include "*.exe", "*.dll", "*.md", "*.txt"
Get-ChildItem $dist -Recurse -File | ForEach-Object { "{0,-50} {1,8:N1} MB" -f $_.FullName.Substring($dist.Length + 1), ($_.Length / 1MB) }
"Ready: $dist\purple.exe"
