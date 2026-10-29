# Build the Windows installer and the winget manifests for a release.
#   pwsh installer/build.ps1 -Tag v0.2.0 [-Repository Flory-1/oxplorer]
# Needs target/release/oxplorer.exe and oxplorer-indexer.exe
# (cargo build --release -p ox-app -p ox-indexer).
# Writes dist/oxplorer-<tag>-setup-x64.exe (+ .sha256) and
# dist/oxplorer-<tag>-winget.zip (manifests to submit to microsoft/winget-pkgs).
param(
    [Parameter(Mandatory = $true)][string]$Tag,
    [string]$Repository = "Flory-1/oxplorer"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$version = $Tag.TrimStart("v")
# Inno Setup wants a numeric version (no "-beta" etc.).
$numeric = ($version -split "[-+]")[0]
$name = "oxplorer-$Tag-setup-x64"
$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null

$iscc = Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) {
    choco install innosetup -y --no-progress | Out-Null
}
& $iscc /Q "/DAppVersion=$numeric" "/DOutputName=$name" (Join-Path $PSScriptRoot "oxplorer.iss")
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed ($LASTEXITCODE)" }

$setup = Join-Path $dist "$name.exe"
$sha = (Get-FileHash $setup -Algorithm SHA256).Hash
"$($sha.ToLower())  $name.exe" | Out-File -Encoding ascii "$setup.sha256"

# winget manifests, filled in for this release.
$manifests = Join-Path $dist "winget"
Remove-Item -Recurse -Force $manifests -ErrorAction SilentlyContinue
$target = Join-Path $manifests "manifests\f\Flory-1\Oxplorer\$version"
New-Item -ItemType Directory -Force -Path $target | Out-Null
$url = "https://github.com/$Repository/releases/download/$Tag/$name.exe"
$date = Get-Date -Format "yyyy-MM-dd"
Get-ChildItem (Join-Path $PSScriptRoot "winget\*.yaml") | ForEach-Object {
    $text = Get-Content $_.FullName -Raw
    $text = $text.Replace("{VERSION}", $version).Replace("{URL}", $url)
    $text = $text.Replace("{SHA256}", $sha).Replace("{DATE}", $date)
    Set-Content -Encoding utf8NoBOM -Path (Join-Path $target $_.Name) -Value $text
}
Compress-Archive -Force -Path (Join-Path $manifests "manifests") -DestinationPath (Join-Path $dist "oxplorer-$Tag-winget.zip")
Write-Host "Built $setup"
