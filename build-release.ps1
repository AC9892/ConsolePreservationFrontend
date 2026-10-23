$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    cargo test --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Inspector tests failed' }
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    Copy-Item -LiteralPath target/release/PS4.exe -Destination dist/PS4.exe -Force
    Copy-Item -LiteralPath target/release/PS4Inspector.exe -Destination dist/PS4Inspector.exe -Force
    Copy-Item -LiteralPath target/release/PS4Trace.exe -Destination dist/PS4Trace.exe -Force
    Copy-Item -LiteralPath target/release/PS4ModuleTest.exe -Destination dist/PS4ModuleTest.exe -Force
    Write-Output "Built $PSScriptRoot\dist\PS4.exe"
} finally { Pop-Location }
