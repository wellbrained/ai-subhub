using System.Text.Json;
using Avalonia.Threading;
using Hub.Core;
using Microsoft.Win32;

namespace Hub.App;

public sealed class HubController : IDisposable
{
    public HubSettings Settings { get; }
    public SettingsStore Store { get; }
    public LaunchService Launcher { get; }
    public Dictionary<Guid, UsageResult> Results { get; } = [];
    public Dictionary<Guid, string> Plans { get; } = [];
    public IReadOnlyDictionary<ProviderKind, DesktopApplication> DesktopApplications { get; private set; } = new Dictionary<ProviderKind, DesktopApplication>();
    private readonly Dictionary<Guid, RefreshSchedule> _schedules = [];
    private readonly HttpClient _http = new() { Timeout = TimeSpan.FromSeconds(25) };
    private readonly CancellationTokenSource _stop = new();
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromSeconds(30) };
    public bool IsRefreshing { get; private set; }
    public bool IsDemo { get; }
    public string? Notice { get; set; }
    public event Action? Changed;

    public HubController(bool demo)
    {
        IsDemo = demo;
        var root = demo ? Path.Combine(Path.GetTempPath(), "ai-subhub-demo") : UserDataPaths.Resolve(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData));
        Store = new SettingsStore(root);
        Settings = demo ? DemoSettings() : Store.Load();
        Notice = Store.LoadWarning;
        Launcher = new(Settings);
        ReadPlans();
        foreach (var cached in Settings.Readings) Results[cached.AccountId] = new(UsageState.Unavailable, "Cached reading. Waiting for refresh.", cached.Snapshot);
        if (demo)
        {
            foreach (var account in Settings.Accounts)
            {
                var used = account.Label.Contains("Personal") ? 32 : 71;
                Results[account.Id] = new(UsageState.Ready, "Connected", new(DateTimeOffset.UtcNow.AddMinutes(-2), [new("5-hour allowance", used, DateTimeOffset.UtcNow.AddHours(2.3)), new("Weekly allowance", used - 12, DateTimeOffset.UtcNow.AddDays(3))]), AccountEmail: "demo@example.test", Resets: account.Provider == ProviderKind.Codex ? new(2, DateTimeOffset.UtcNow.AddDays(3)) : null);
            }
        }
    }

    public void Start()
    {
        _ = DiscoverDesktopAppsAsync();
        if (!IsDemo && Settings.StartWithWindows)
        {
            try { SetStartup(true); }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException or System.Security.SecurityException) { Notice = "Windows startup could not be updated. Disable and re-enable it in Settings → General."; }
        }
        _timer.Tick += async (_, _) => { await RefreshAsync(false); Changed?.Invoke(); };
        _timer.Start();
        _ = RefreshAsync(true);
    }

    private async Task DiscoverDesktopAppsAsync()
    {
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(_stop.Token);
        timeout.CancelAfter(TimeSpan.FromSeconds(15));
        try { DesktopApplications = await DesktopApps.DiscoverAsync(timeout.Token); }
        catch (Exception ex) when (ex is IOException or JsonException or OperationCanceledException or System.ComponentModel.Win32Exception) { /* Undetected desktop apps remain unavailable in the launch menu. */ }
        Changed?.Invoke();
    }

    public string SelectedDirectory => Settings.Projects.FirstOrDefault(p => p.Id == Settings.SelectedProjectId)?.Directory ?? Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);

    public void Save()
    {
        ReadPlans();
        if (!IsDemo) Store.Save(Settings);
        Changed?.Invoke();
    }

    public void UpdateTheme(string theme)
    {
        Settings.Theme = theme;
        ((App)Avalonia.Application.Current!).SetTheme(theme);
        Save();
    }

    public void SetStartup(bool enabled)
    {
        if (IsDemo) return;
        if (!OperatingSystem.IsWindows()) throw new InvalidOperationException("Windows startup is supported only on Windows.");
        var executable = Environment.ProcessPath ?? throw new InvalidOperationException("The app executable path could not be found.");
        if (Path.GetFileNameWithoutExtension(executable).Equals("dotnet", StringComparison.OrdinalIgnoreCase)) throw new InvalidOperationException("Enable startup from the portable executable, rather than dotnet run.");
        using var key = Registry.CurrentUser.CreateSubKey(@"Software\Microsoft\Windows\CurrentVersion\Run");
        key.DeleteValue("AISubscriptionHub", false); // Migration-only cleanup of the previous startup identifier.
        if (enabled) key.SetValue("AISubHub", $"\"{executable}\" --tray");
        else key.DeleteValue("AISubHub", false);
        Settings.StartWithWindows = enabled;
        Save();
    }

    public async Task RefreshAsync(bool force)
    {
        if (IsRefreshing || IsDemo || _stop.IsCancellationRequested) return;
        ReadPlans();
        IsRefreshing = true; Changed?.Invoke();
        try
        {
            foreach (var account in Settings.Accounts.ToArray())
            {
                if (_stop.IsCancellationRequested) break;
                if (!_schedules.TryGetValue(account.Id, out var schedule)) _schedules[account.Id] = schedule = new();
                // Manual refresh can bypass ordinary cadence, but respects provider rate limiting.
                if (DateTimeOffset.UtcNow < schedule.NextAttempt && (!force || Results.GetValueOrDefault(account.Id)?.State == UsageState.RateLimited)) continue;
                UsageResult result;
                using var timeout = CancellationTokenSource.CreateLinkedTokenSource(_stop.Token);
                timeout.CancelAfter(TimeSpan.FromSeconds(35));
                try
                {
                    IUsageProvider provider = account.Provider == ProviderKind.Claude ? new ClaudeUsageProvider(_http, Launcher) : new CodexUsageProvider(Launcher);
                    result = await provider.ReadAsync(account, timeout.Token);
                }
                catch (OperationCanceledException) { result = new(UsageState.Unavailable, "Usage request timed out. Automatic refresh will retry."); }
                catch (HttpRequestException) { result = new(UsageState.Unavailable, "Could not reach the provider. Check your internet connection."); }
                catch (JsonException) { result = new(UsageState.Unavailable, "The provider returned an unrecognized response. Check the CLI version and try again."); }
                catch (UnauthorizedAccessException) { result = new(UsageState.Unavailable, "The profile could not be read. Check its folder permissions."); }
                catch (Exception ex) when (ex is IOException or InvalidOperationException or System.ComponentModel.Win32Exception) { result = new(UsageState.Unavailable, "Could not read usage. Check the profile login and CLI installation, then refresh."); }
                if (!Settings.Accounts.Contains(account)) continue;
                schedule.Complete(result, DateTimeOffset.UtcNow, Settings.RefreshMinutes);
                if (result.Snapshot is null && Results.TryGetValue(account.Id, out var previous)) result = result with { Snapshot = previous.Snapshot };
                Results[account.Id] = result;
                Changed?.Invoke();
            }
            Settings.Readings = Results.Where(x => x.Value.Snapshot is not null && Settings.Accounts.Any(a => a.Id == x.Key)).Select(x => new CachedReading(x.Key, x.Value.Snapshot!)).ToList();
            Save();
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { Notice = "Usage was read, but local settings could not be saved. Check disk space and folder permissions."; }
        finally { IsRefreshing = false; Changed?.Invoke(); }
    }

    public void RemoveAccount(Guid id)
    {
        Settings.Accounts.RemoveAll(a => a.Id == id);
        Settings.Readings.RemoveAll(a => a.AccountId == id);
        Results.Remove(id); _schedules.Remove(id); Save();
    }

    public void ForgetReading(Guid id) { Results.Remove(id); Settings.Readings.RemoveAll(x => x.AccountId == id); _schedules.Remove(id); }
    private void ReadPlans()
    {
        Plans.Clear();
        foreach (var account in Settings.Accounts)
            Plans[account.Id] = IsDemo ? account.Provider == ProviderKind.Claude ? "Pro" : "Plus" : SubscriptionPlan.Read(account);
    }
    public void Dispose() { _timer.Stop(); _stop.Cancel(); _stop.Dispose(); _http.Dispose(); }

    private static HubSettings DemoSettings() => new()
    {
        Accounts = [new(Guid.NewGuid(), ProviderKind.Claude, "Personal", @"C:\Profiles\claude-personal"), new(Guid.NewGuid(), ProviderKind.Claude, "Second subscription", @"C:\Profiles\claude-second"), new(Guid.NewGuid(), ProviderKind.Codex, "Personal", @"C:\Profiles\codex")],
        Projects = [new(Guid.NewGuid(), "AI SubHub", @"D:\Github\ai-subhub"), new(Guid.NewGuid(), "Website", @"D:\Demo\website"), new(Guid.NewGuid(), "Later", @"D:\Demo\later", true)]
    };
}
