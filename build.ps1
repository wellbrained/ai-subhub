param([ValidateSet('Check', 'Publish', 'PublishMac')][string]$Task = 'Check', [string]$PythonExecutable = 'python')
$ErrorActionPreference = 'Stop'
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_NOLOGO = '1'
$dotnet = Join-Path $PSScriptRoot '.tools\dotnet\dotnet.exe'
if (-not (Test-Path -LiteralPath $dotnet)) { $dotnet = (Get-Command dotnet -ErrorAction Stop).Source }
Push-Location $PSScriptRoot
try {
    & $dotnet build AI.SubHub.slnx -c Release --nologo
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    & $dotnet test AI.SubHub.slnx -c Release --no-build --nologo
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
    if ($Task -eq 'PublishMac') {
        $version = ([xml](Get-Content -LiteralPath 'Directory.Build.props' -Raw)).Project.PropertyGroup.Version
        foreach ($rid in @('osx-arm64', 'osx-x64')) {
            $stamp = Get-Date -Format 'yyyyMMddHHmmss'
            $published = Join-Path $PSScriptRoot "artifacts/publish-$version-$rid-$stamp"
            $output = Join-Path $PSScriptRoot "artifacts/AI-SubHub-$version-$rid-unsigned-$stamp"
            & $dotnet publish src/Hub.App/Hub.App.csproj -c Release -r $rid --self-contained true -p:DebugType=None -o $published
            if ($LASTEXITCODE -ne 0) { throw "Publish failed: $rid" }
            Copy-Item README.md,LICENSE -Destination $published -Force
            & $PythonExecutable scripts/package-macos.py $published $output $version src/Hub.App/Assets/hub.icns
            if ($LASTEXITCODE -ne 0) { throw "Mac packaging failed: $rid" }
        }
    }
    if ($Task -eq 'Publish') {
        $version = ([xml](Get-Content -LiteralPath 'Directory.Build.props' -Raw)).Project.PropertyGroup.Version
        $output = Join-Path $PSScriptRoot "artifacts\AI-SubHub-$version-win-x64"
        & $dotnet publish src/Hub.App/Hub.App.csproj -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:DebugType=None -o $output
        if ($LASTEXITCODE -ne 0) { throw 'Publish failed.' }
        foreach ($symbols in @('libHarfBuzzSharp.pdb', 'libSkiaSharp.pdb')) {
            $symbolPath = Join-Path $output $symbols
            if (Test-Path -LiteralPath $symbolPath) { Remove-Item -LiteralPath $symbolPath }
        }
        Copy-Item README.md,LICENSE -Destination $output -Force
        Compress-Archive -Path "$output\*" -DestinationPath "artifacts\AI-SubHub-$version-win-x64.zip" -Force
        @{ version = [string]$version } | ConvertTo-Json | Set-Content -LiteralPath 'artifacts\current-production.json' -Encoding UTF8
        Write-Host "Portable app: $output\AI.SubHub.exe"
    }
}
finally { Pop-Location }
