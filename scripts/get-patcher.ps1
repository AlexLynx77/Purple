# Fetches the LTK patcher (ltk_patcher_host.exe + ltk_patcher_dll.dll) from the LTK Manager repository.
#
# Purple does not ship these two files: they are LeagueToolkit's closed-source binaries (see
# vendor/ltk-patcher/LTK-PATCHER-LICENSE.md). They must come as a matching pair from the same
# release, and they stop working on game versions newer than their built-in end-of-life date,
# so run this again when Purple reports that the patcher is too old.
#
#   .\scripts\get-patcher.ps1                        # latest stable LTK Manager release, into vendor\ltk-patcher
#   .\scripts\get-patcher.ps1 -Ref main              # newest files, possibly not released yet
#   .\scripts\get-patcher.ps1 -Destination C:\x      # somewhere else
#
# What was fetched (release, date, SHA-256) is written to PATCHER-VERSION.txt next to the files.
param(
    [string]$Destination = (Join-Path (Split-Path $PSScriptRoot -Parent) "vendor\ltk-patcher"),
    [string]$Ref = "latest"
)

$ErrorActionPreference = "Stop"
if ($Ref -eq "latest") {
    $Ref = (Invoke-RestMethod "https://api.github.com/repos/LeagueToolkit/ltk-manager/releases/latest" -UseBasicParsing).tag_name
}
"Fetching the patcher from LeagueToolkit/ltk-manager@$Ref"
$source = "https://github.com/LeagueToolkit/ltk-manager/raw/$Ref"
New-Item -ItemType Directory -Force $Destination | Out-Null

foreach ($file in "ltk_patcher_host.exe", "ltk_patcher_dll.dll") {
    $part = Join-Path $Destination "$file.part"
    Invoke-WebRequest "$source/src-tauri/resources/$file" -OutFile $part -UseBasicParsing
    $bytes = [IO.File]::ReadAllBytes($part)
    # Refuse anything that is not a Windows executable (an HTML error page, for instance).
    if ($bytes.Length -lt 100KB -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
        Remove-Item $part
        throw "$file did not download as a Windows binary"
    }
    Move-Item $part (Join-Path $Destination $file) -Force
    "{0}  {1:N0} bytes" -f $file, $bytes.Length
}

Invoke-WebRequest "$source/LTK-PATCHER-LICENSE.md" -OutFile (Join-Path $Destination "LTK-PATCHER-LICENSE.md") -UseBasicParsing
$version = @("source:  LeagueToolkit/ltk-manager@$Ref", "fetched: $(Get-Date -Format 'yyyy-MM-dd')")
foreach ($file in "ltk_patcher_host.exe", "ltk_patcher_dll.dll") {
    $version += "{0}  {1}" -f (Get-FileHash (Join-Path $Destination $file) -Algorithm SHA256).Hash.ToLower(), $file
}
$version | Set-Content (Join-Path $Destination "PATCHER-VERSION.txt") -Encoding ascii
"Patcher ready in $Destination"
