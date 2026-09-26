# Downloads the custom FFmpeg build (see scripts/build-ffmpeg.sh) into
# third_party/ffmpeg. Requires the GitHub CLI logged in with access to the repo.
param(
    [string]$Tag = ""
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "third_party\ffmpeg"
$tmp = Join-Path $env:TEMP "geniusclip-ffmpeg"

if (-not $Tag) {
    # Filtered in PowerShell: Windows PowerShell mangles quotes in --jq args.
    $Tag = (gh release list --repo GandalfDark/geniusclip --limit 50 --json tagName | ConvertFrom-Json |
        Where-Object { $_.tagName -like 'ffmpeg-*' } | Select-Object -First 1).tagName
}
if (-not $Tag) { throw "No ffmpeg-* release found. Run the 'Build FFmpeg' workflow first." }

Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $tmp | Out-Null
gh release download $Tag --repo GandalfDark/geniusclip --pattern "ffmpeg-geniusclip-win64.zip" --dir $tmp
Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue
Expand-Archive (Join-Path $tmp "ffmpeg-geniusclip-win64.zip") -DestinationPath $dest
Get-Content (Join-Path $dest "BUILDINFO.txt")
Write-Host "FFmpeg $Tag installed to $dest"
