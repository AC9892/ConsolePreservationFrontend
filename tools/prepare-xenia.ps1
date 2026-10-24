[CmdletBinding()]
param(
    [string]$SourceDirectory = (Join-Path (Split-Path -Parent $PSScriptRoot) 'build\xenia-canary'),
    [switch]$CheckOnly
)

$ErrorActionPreference = 'Stop'
$revision = '2372a54105d9d598ab5bc7ba94a72ef1bea4b271'
$projectRoot = Split-Path -Parent $PSScriptRoot
$patchPath = Join-Path $projectRoot 'integrations\xenia-canary\cpf-dashboard.patch'
$sourcePath = [System.IO.Path]::GetFullPath($SourceDirectory)

if (-not (Test-Path -LiteralPath $patchPath -PathType Leaf)) {
    throw 'The canonical CPF dashboard patch is missing.'
}
if (-not (Test-Path -LiteralPath $sourcePath)) {
    if ($CheckOnly) { throw 'CheckOnly requires an existing source checkout.' }
    & git clone --no-checkout https://github.com/xenia-canary/xenia-canary.git $sourcePath
    if ($LASTEXITCODE -ne 0) { throw 'Upstream clone failed.' }
    & git -C $sourcePath checkout --detach $revision
    if ($LASTEXITCODE -ne 0) { throw 'Pinned revision checkout failed.' }
    & git -C $sourcePath submodule update --init --recursive
    if ($LASTEXITCODE -ne 0) { throw 'Dependency checkout failed.' }
}

$actualRevision = & git -C $sourcePath rev-parse HEAD
if ($LASTEXITCODE -ne 0 -or $actualRevision.Trim() -ne $revision) {
    throw 'The selected checkout is not at the pinned upstream revision. Nothing was reset or overwritten.'
}

& git -C $sourcePath apply --check $patchPath 2>$null
if ($LASTEXITCODE -eq 0) {
    if ($CheckOnly) {
        Write-Output 'Patch check passed. The source checkout was not modified.'
        return
    }
    & git -C $sourcePath apply $patchPath
    if ($LASTEXITCODE -ne 0) { throw 'Patch application failed.' }
    Write-Output 'CPF dashboard patch applied. Follow the README build instructions.'
    return
}

& git -C $sourcePath apply --reverse --check $patchPath 2>$null
if ($LASTEXITCODE -eq 0) {
    Write-Output 'All CPF patch hunks are already present. No files were changed.'
    return
}
throw 'The patch does not apply cleanly and is not fully present. No files were changed; inspect local changes before proceeding.'
