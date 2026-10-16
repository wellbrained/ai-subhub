param([ValidateSet('Check', 'Publish')][string]$Task = 'Check')
$ErrorActionPreference = 'Stop'
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_NOLOGO = '1'
$dotnet = Join-Path $PSScriptRoot '.tools\dotnet\dotnet.exe'
if (-not (Test-Path -LiteralPath $dotnet)) { $dotnet = (Get-Command dotnet -ErrorAction Stop).Source }
Push-Location $PSScriptRoot
try {
    & $dotnet build AI.SubscriptionHub.slnx -c Release --nologo
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    & $dotnet test AI.SubscriptionHub.slnx -c Release --no-build --nologo
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
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
