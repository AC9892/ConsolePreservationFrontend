$ErrorActionPreference = 'Stop'
python (Join-Path $PSScriptRoot 'prepare-public.py')
if ($LASTEXITCODE -ne 0) { throw 'Public preparation failed; do not upload its staging files.' }
