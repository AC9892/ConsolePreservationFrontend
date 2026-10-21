$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $MyInvocation.MyCommand.Path
$workspace = Split-Path -Parent $project
$output = Join-Path $project 'bin'
New-Item -ItemType Directory -Path $output -Force | Out-Null

& (Join-Path $project 'src-tauri\icons\create-dsi-icon.ps1')

Push-Location $project
try {
    if (-not (Test-Path -LiteralPath (Join-Path $project 'node_modules\.bin\vite.cmd'))) {
        npm ci --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
    }
    npm run build
    if ($LASTEXITCODE -ne 0) { throw 'TypeScript/Vite build failed' }
    cargo build -p cpf-desktop --bin cpf-desktop -p cpf-runtime-worker --release --offline
    if ($LASTEXITCODE -ne 0) { throw 'Rust/Tauri build failed' }
    Copy-Item -LiteralPath (Join-Path $project 'target\release\cpf-desktop.exe') `
        -Destination (Join-Path $output 'Console Manager.exe') -Force
    $workerBuilt = Join-Path $project 'target\release\cpf-runtime-worker.exe'
    $workerOutput = Join-Path $output 'DSi Backend.exe'
    if (-not (Test-Path -LiteralPath $workerOutput) -or
        (Get-FileHash -LiteralPath $workerBuilt -Algorithm SHA256).Hash -ne
        (Get-FileHash -LiteralPath $workerOutput -Algorithm SHA256).Hash) {
        Copy-Item -LiteralPath $workerBuilt -Destination $workerOutput -Force
    }

    $env:TAURI_CONFIG = '{"productName":"DSi","identifier":"org.consolepreservation.dsi","bundle":{"icon":["icons/dsi.ico"]}}'
    try {
        cargo build -p cpf-desktop --bin cpf-dsi --release --offline
        if ($LASTEXITCODE -ne 0) { throw 'DSi app build failed' }
        Copy-Item -LiteralPath (Join-Path $project 'target\release\cpf-dsi.exe') `
            -Destination (Join-Path $output 'Development_DSI.exe') -Force
    } finally {
        Remove-Item Env:TAURI_CONFIG -ErrorAction SilentlyContinue
    }
} finally {
    Pop-Location
}

foreach ($oldName in 'CPF Console Manager.exe', 'CPF DSi.exe', 'CPF Runtime Worker.exe') {
    $oldPath = Join-Path $output $oldName
    if (Test-Path -LiteralPath $oldPath) { Remove-Item -LiteralPath $oldPath }
}
Write-Host "Built $(Join-Path $output 'Console Manager.exe') and $(Join-Path $output 'Development_DSI.exe')"
Write-Host "Workspace: $workspace"
