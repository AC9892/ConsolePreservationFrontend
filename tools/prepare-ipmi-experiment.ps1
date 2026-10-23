param(
    [Parameter(Mandatory=$true)][string]$DumpRoot,
    [Parameter(Mandatory=$true)][string]$Backend,
    [Parameter(Mandatory=$true)][string]$SandboxParent
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$hostExe = Join-Path $project 'dist/PS4.exe'
$json = & $hostExe --prepare-shellui $DumpRoot $Backend $SandboxParent | Out-String
if ($LASTEXITCODE -ne 0) { throw 'Fresh sandbox preparation failed' }
$plan = $json | ConvertFrom-Json
if ($plan.backend_profile -ne 'shadps4-v0.20.0') { throw 'This experiment requires the pinned v0.20.0 backend' }
$library = Join-Path $plan.sandbox 'user/sys_modules/libSceIpmi.sprx'
$expected = 'c9081d79493740a55fc3d40e160e906c17879abeca47a9315188656113853ae5'
if ((Get-FileHash -LiteralPath $library -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
    throw 'IPMI module differs from the researched dump; no override installed'
}
$target = Join-Path $plan.sandbox 'user/sys_modules/NPXS20001'
New-Item -ItemType Directory -Path $target | Out-Null
Copy-Item -LiteralPath $library -Destination (Join-Path $target 'libSceIpmi.sprx')
$plan | Add-Member -NotePropertyName experiment -NotePropertyValue 'v0.20 baseline plus native IPMI per-title LLE override only'
$plan | Add-Member -NotePropertyName lle_overrides -NotePropertyValue @(@{module='libSceIpmi.sprx';sha256=$expected})
$plan | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $plan.sandbox 'plan.json') -Encoding utf8NoBOM
$plan | ConvertTo-Json -Depth 8
