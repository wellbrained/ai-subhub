using System.Diagnostics;
using System.Text.Json;

namespace Hub.Core;

public sealed record DesktopApplication(string AppId)
{
    public void Launch()
    {
        if (OperatingSystem.IsMacOS() && Directory.Exists(AppId) && AppId.EndsWith(".app", StringComparison.Ordinal))
        {
            Process.Start(MacPlatform.Open(AppId))?.Dispose(); return;
        }
        if (AppId.Contains('!') && !Path.IsPathRooted(AppId))
        {
            var start = new ProcessStartInfo("explorer.exe") { UseShellExecute = false };
            start.ArgumentList.Add("shell:AppsFolder\\" + AppId);
            Process.Start(start)?.Dispose();
        }
        else if (File.Exists(AppId) && Path.GetExtension(AppId).Equals(".exe", StringComparison.OrdinalIgnoreCase))
            Process.Start(new ProcessStartInfo(AppId) { UseShellExecute = true })?.Dispose();
        else throw new InvalidOperationException("The desktop app is no longer installed. Restart AI SubHub to detect installed apps again.");
    }
}

public static class DesktopApps
{
    public static async Task<IReadOnlyDictionary<ProviderKind, DesktopApplication>> DiscoverAsync(CancellationToken ct)
    {
        if (OperatingSystem.IsMacOS())
        {
            var found = new Dictionary<ProviderKind, DesktopApplication>();
            foreach (var provider in Enum.GetValues<ProviderKind>())
                if (MacPlatform.AppBundle(provider.ToString()) is { } bundle) found[provider] = new(bundle);
            return found;
        }
        // Query Windows registration, not CLI credentials or application databases.
        const string script = """
            $ErrorActionPreference = 'SilentlyContinue'
            $apps = @(Get-StartApps)
            $claude = ($apps | Where-Object Name -eq 'Claude' | Select-Object -First 1).AppID
            $codex = ($apps | Where-Object Name -eq 'Codex' | Select-Object -First 1).AppID
            if (-not $codex) {
                $package = Get-AppxPackage -Name 'OpenAI.Codex' | Select-Object -First 1
                if ($package) { $codex = $package.PackageFamilyName + '!App' }
            }
            [pscustomobject]@{ Claude = $claude; Codex = $codex } | ConvertTo-Json -Compress
            """;
        var start = new ProcessStartInfo("powershell.exe") { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true };
        foreach (var arg in new[] { "-NoLogo", "-NoProfile", "-EncodedCommand", SubscriptionEnvironment.Encode(script) }) start.ArgumentList.Add(arg);
        using var process = Process.Start(start) ?? throw new IOException("Desktop app detection could not start.");
        process.ErrorDataReceived += (_, _) => { }; process.BeginErrorReadLine();
        try
        {
            var output = await process.StandardOutput.ReadToEndAsync(ct);
            await process.WaitForExitAsync(ct);
            using var json = JsonDocument.Parse(output);
            var apps = new Dictionary<ProviderKind, DesktopApplication>();
            foreach (var provider in Enum.GetValues<ProviderKind>())
                if (json.RootElement.TryGetProperty(provider.ToString(), out var id) && id.ValueKind == JsonValueKind.String && !string.IsNullOrWhiteSpace(id.GetString())) apps[provider] = new(id.GetString()!);
            return apps;
        }
        finally { if (!process.HasExited) process.Kill(entireProcessTree: true); }
    }
}
