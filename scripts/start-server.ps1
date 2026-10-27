param([Parameter(Mandatory = $true)][ValidateRange(1024,65535)][int]$Port)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$release = Get-Content -LiteralPath (Join-Path $projectRoot 'artifacts\current-production.json') -Raw | ConvertFrom-Json
if ($release.version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid production release pointer. Run build.ps1 -Task Publish.' }
$executable = Join-Path $projectRoot "artifacts\AI-SubHub-$($release.version)-win-x64\AI.SubHub.exe"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw 'Published production executable is missing.' }
# CoreHub starts this outside its Task Scheduler job; the app remains in that process tree for Stop/Restart.
$app = Start-Process -FilePath $executable -ArgumentList @('--tray','--health-port',[string]$Port) -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru
$app.WaitForExit()
exit $app.ExitCode
