using System.Text.Json;
using Hub.Core;

namespace Hub.App;

internal static class SmokeTest
{
    public static async Task<int> Run(string[] args)
    {
        string? Option(string name) { var index = Array.IndexOf(args, name); return index >= 0 && index + 1 < args.Length ? args[index + 1] : null; }
        var report = Option("--report");
        if (report is null || !Enum.TryParse<ProviderKind>(Option("--provider"), out var provider) || Option("--profile") is not { } path) return 2;
        var settings = new HubSettings();
        var launcher = new LaunchService(settings);
        using var http = new HttpClient { Timeout = TimeSpan.FromSeconds(25) };
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(40));
        try
        {
            IUsageProvider adapter = provider == ProviderKind.Claude ? new ClaudeUsageProvider(http, launcher) : new CodexUsageProvider(launcher);
            var result = await adapter.ReadAsync(new(Guid.NewGuid(), provider, "Smoke test", path), timeout.Token);
            // Deliberately exclude account identity, credential contents and raw provider responses.
            await File.WriteAllTextAsync(report, JsonSerializer.Serialize(new { Provider = provider.ToString(), State = result.State.ToString(), result.Message, WindowCount = result.Snapshot?.Windows.Count ?? 0, HasAccountEmail = result.AccountEmail is not null, AvailableResets = result.Resets?.AvailableCount }, new JsonSerializerOptions { WriteIndented = true }));
            return result.State == UsageState.Ready ? 0 : 1;
        }
        catch (Exception ex) when (ex is IOException or HttpRequestException or JsonException or InvalidOperationException or OperationCanceledException or System.ComponentModel.Win32Exception)
        {
            await File.WriteAllTextAsync(report, JsonSerializer.Serialize(new { Provider = provider.ToString(), State = "Error", ErrorType = ex.GetType().Name }));
            return 1;
        }
    }
}
