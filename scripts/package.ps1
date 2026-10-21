# Builds Purple and puts the ready-to-run program in dist\
#
#   .\scripts\package.ps1              # build (release), then copy
#   .\scripts\package.ps1 -SkipBuild   # only copy the last build
#
# Purple is a single program, dist\purple.exe. It downloads LTK Patcher by itself on first run.
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

$dist = Join-Path $root "dist"
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item (Join-Path $root "target\release\purple.exe") $dist
$exe = Get-Item (Join-Path $dist "purple.exe")
"{0}  {1:N1} MB" -f $exe.FullName, ($exe.Length / 1MB)
