$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$cargoToml = Get-Content -Path (Join-Path $root 'Cargo.toml') -Raw
if ($cargoToml -notmatch '(?m)^version\s*=\s*"(.+)"') {
    throw 'Could not read version from Cargo.toml [package] section.'
}
$version = $Matches[1]

& cargo build --release
if ($LASTEXITCODE -ne 0) {
    throw "cargo build --release failed with exit code $LASTEXITCODE"
}

$exe = Join-Path $root 'target\release\ai-usage-widget.exe'
if (-not (Test-Path -LiteralPath $exe)) {
    throw "Expected release exe not found: $exe"
}

$exeText = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($exe))
if ($exeText -match '(?i)vcruntime140\.dll') {
    throw 'Release exe still depends on the dynamic VC runtime'
}

$name = "AI-Usage-Widget-$version-portable"
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist $name
$zip = Join-Path $dist "$name.zip"

New-Item -ItemType Directory -Path $dist -Force | Out-Null
if (Test-Path -LiteralPath $stage) {
    Remove-Item -LiteralPath $stage -Recurse -Force
}
if (Test-Path -LiteralPath $zip) {
    Remove-Item -LiteralPath $zip -Force
}
New-Item -ItemType Directory -Path $stage -Force | Out-Null

Copy-Item -LiteralPath $exe -Destination $stage
Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination $stage

Compress-Archive -Path $stage -DestinationPath $zip
Remove-Item -LiteralPath $stage -Recurse -Force

Write-Output $zip
