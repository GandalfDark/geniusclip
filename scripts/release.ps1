# Builds the installer and publishes it as a release of the public releases
# repo, together with latest.json for the in-app updater.
#
#   .\scripts\release.ps1 -Notes "Что нового"            # build + publish
#   .\scripts\release.ps1 -Notes "..." -BuildOnly        # build, don't publish
#
# The version comes from src-tauri/tauri.conf.json (bump it first; keep
# Cargo.toml and package.json in sync). Needs the GitHub CLI logged in as the
# owner of the releases repo and the updater key in ~/.tauri/geniusclip.key.
param(
    [Parameter(Mandatory)][string]$Notes,
    [switch]$BuildOnly
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$repo = "GandalfDark/geniusclip-releases"
$version = (Get-Content (Join-Path $root "src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json).version
$tag = "v$version"

if (-not $BuildOnly) {
    gh release view $tag --repo $repo *> $null
    if ($LASTEXITCODE -eq 0) { throw "Release $tag already exists: bump the version first." }
}

$keyPath = Join-Path $env:USERPROFILE ".tauri\geniusclip.key"
if (-not (Test-Path $keyPath)) { throw "Updater signing key not found: $keyPath" }
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $keyPath -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""

Push-Location $root
try {
    npm run tauri build
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
} finally {
    Pop-Location
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
}

$bundle = Join-Path $root "target\release\bundle\nsis"
$setup = Get-Item (Join-Path $bundle "GeniusClip_${version}_x64-setup.exe")
$signature = (Get-Content "$($setup.FullName).sig" -Raw).Trim()
$latest = [ordered]@{
    version   = $version
    notes     = $Notes
    pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = @{
        "windows-x86_64" = @{
            signature = $signature
            url       = "https://github.com/$repo/releases/download/$tag/$($setup.Name)"
        }
    }
}
$latestPath = Join-Path $bundle "latest.json"
# UTF-8 without BOM: the updater parses this file as JSON.
[IO.File]::WriteAllText($latestPath, ($latest | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))

Write-Host "Built $($setup.Name) ($([math]::Round($setup.Length / 1MB, 1)) MB)"
if ($BuildOnly) {
    Write-Host "Not published (-BuildOnly). Installer: $($setup.FullName)"
    return
}
gh release create $tag $setup.FullName $latestPath --repo $repo --title "GeniusClip $version" --notes $Notes
if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
Write-Host "Published https://github.com/$repo/releases/tag/$tag"
