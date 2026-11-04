[CmdletBinding()]
param(
    [string]$SourceDirectory = (Join-Path (Split-Path -Parent $PSScriptRoot) 'build\xenia-canary'),
    [switch]$CheckOnly
)

$ErrorActionPreference = 'Stop'
$revision = '2372a54105d9d598ab5bc7ba94a72ef1bea4b271'
$projectRoot = Split-Path -Parent $PSScriptRoot
$patchPaths = @('cpf-dashboard.patch', 'cpf-gamer-tile.patch', 'cpf-default-system-image.patch') | ForEach-Object { Join-Path $projectRoot ('patches\' + $_) }
$sourcePath = [System.IO.Path]::GetFullPath($SourceDirectory)

foreach ($patch in $patchPaths) {
    if (-not (Test-Path -LiteralPath $patch -PathType Leaf)) { throw "Missing patch: $patch" }
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

$arguments = @((Join-Path $PSScriptRoot 'prepare_xenia.py'), $sourcePath) + $patchPaths
if ($CheckOnly) { $arguments += '--check' }
& python @arguments
if ($LASTEXITCODE -ne 0) { throw 'Public source preparation failed. See the error above; no reset was performed.' }
