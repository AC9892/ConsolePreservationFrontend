$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
Push-Location $project
try {
    $env:CARGO_NET_OFFLINE = 'true'
    cargo build --release --bin Xbox
    if ($LASTEXITCODE -ne 0) { throw "Xbox build failed with exit code $LASTEXITCODE" }
    Copy-Item -LiteralPath 'target\release\Xbox.exe' -Destination 'Xbox.exe' -Force
    Write-Output "Built $(Join-Path $project 'Xbox.exe')"
} finally {
    Pop-Location
}
