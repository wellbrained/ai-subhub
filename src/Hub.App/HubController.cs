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
    internal event Action<IReadOnlyList<BillingReminder>>? BillingRemindersDue;

    private readonly Func<AccountProfile, CancellationToken, Task<UsageResult>> _readUsage;
    private readonly Action<AccountProfile, string> _launchSignIn;

    public HubController(bool demo) : this(demo, null, null, null) { }

    internal HubController(bool demo, SettingsStore? store,
        Func<AccountProfile, CancellationToken, Task<UsageResult>>? readUsage,
        Action<AccountProfile, string>? launchSignIn)
    {
        IsDemo = demo;
        var root = demo ? Path.Combine(Path.GetTempPath(), "ai-subhub-demo") : UserDataPaths.Current();
        Store = store ?? new SettingsStore(root);
        Settings = demo ? DemoSettings() : Store.Load();
        Notice = Store.LoadWarning;
        Launcher = new(Settings);
        _readUsage = readUsage ?? ((account, ct) =>
        {
            IUsageProvider provider = account.Provider == ProviderKind.Claude ? new ClaudeUsageProvider(_http, Launcher) : new CodexUsageProvider(Launcher);
            return provider.ReadAsync(account, ct);
        });
        _launchSignIn = launchSignIn ?? ((account, completion) => Launcher.Launch(account, Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), login: true, completionFile: completion));
        ReadPlans();
        foreach (var cached in Settings.Readings) Results[cached.AccountId] = new(UsageState.Unavailable, "Cached reading. Waiting for refresh.", cached.Snapshot);
        if (demo)
        {
            foreach (var account in Settings.Accounts)
            {
                var used = account.Label.Contains("Personal") ? 32 : 71;
                Results[account.Id] = new(UsageState.Ready, "Connected", new(DateTimeOffset.UtcNow.AddMinutes(-2), [new("5-hour allowance", used, null), new("Weekly allowance", used - 12, DateTimeOffset.UtcNow.AddDays(3)), new("Weekly · Sonnet allowance", used - 12, DateTimeOffset.UtcNow.AddDays(3))]), AccountEmail: "demo@example.test", Resets: account.Provider == ProviderKind.Codex ? new(2, DateTimeOffset.UtcNow.AddDays(3)) : null);
            }
        }
    }

    public void Start()
    {
        _ = DiscoverDesktopAppsAsync();
        if (!IsDemo && Settings.StartWithWindows)
        {
            try { SetStartup(true); }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException or System.Security.SecurityException) { Notice = "Login startup could not be updated. Disable and re-enable it in Settings → General."; }
        }
        _timer.Tick += async (_, _) => { await RefreshAsync(false); CheckBillingReminders(DateOnly.FromDateTime(DateTime.Now)); Changed?.Invoke(); };
        _timer.Start();
        CheckBillingReminders(DateOnly.FromDateTime(DateTime.Now));
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

        var executable = Environment.ProcessPath ?? throw new InvalidOperationException("The app executable path could not be found.");
        if (Path.GetFileNameWithoutExtension(executable).Equals("dotnet", StringComparison.OrdinalIgnoreCase)) throw new InvalidOperationException("Enable startup from the portable executable, rather than dotnet run.");
        if (OperatingSystem.IsMacOS())
        {
            MacPlatform.SetStartup(enabled, executable); Settings.StartWithWindows = enabled; Save(); return;
        }
        if (!OperatingSystem.IsWindows()) throw new InvalidOperationException("Login startup is not supported on this platform.");
        using var key = Registry.CurrentUser.CreateSubKey(@"Software\Microsoft\Windows\CurrentVersion\Run");
        key.DeleteValue("AISubscriptionHub", false); // Migration-only cleanup of the previous startup identifier.
        if (enabled) key.SetValue("AISubHub", $"\"{executable}\" --tray");
        else key.DeleteValue("AISubHub", false);
        Settings.StartWithWindows = enabled;
        Save();
    }

    public void SignIn(AccountProfile account)
    {
        if (IsDemo || _stop.IsCancellationRequested) return;
        var completion = new LoginCompletion();
        try { _launchSignIn(account, completion.FilePath); }
        catch { completion.Dispose(); throw; }
        Notice = "Complete sign-in in the terminal. Usage will refresh automatically.";
        Changed?.Invoke();
        _ = CompleteSignInAsync(account, completion, _stop.Token);
    }

    private async Task CompleteSignInAsync(AccountProfile account, LoginCompletion completion, CancellationToken ct)
    {
        using (completion)
        {
            try
            {
                var code = await completion.WaitAsync(ct);
                if (code != 0)
                {
                    Notice = code is null ? "Sign-in was not confirmed. Finish it in the terminal, then refresh." : "Sign-in did not complete successfully. Check the terminal and try again.";
                    Changed?.Invoke(); return;
                }
                // A login can finish during an existing usage read. Wait for that
                // read rather than letting its in-flight guard drop this request.
                while (IsRefreshing) await Task.Delay(100, ct);
                var current = Settings.Accounts.FirstOrDefault(a => a.Id == account.Id);
                if (current is null || current.Provider != account.Provider || current.Directory != account.Directory) return;
                ct.ThrowIfCancellationRequested();
                Notice = null;
                if (Results.TryGetValue(account.Id, out var beforeLogin)) Results[account.Id] = beforeLogin with { PlanLabel = null, PlanFromProvider = false, AccountEmail = null };
                await RefreshAsync(true, account.Id);
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested) { }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                Notice = "Sign-in completed, but its result could not be read. Refresh usage manually.";
                Changed?.Invoke();
            }
        }
    }

    public async Task RefreshAsync(bool force, Guid? accountId = null)
    {
        if (IsRefreshing || IsDemo || _stop.IsCancellationRequested) return;
        ReadPlans();
        IsRefreshing = true; Changed?.Invoke();
        try
        {
            foreach (var account in Settings.Accounts.Where(a => accountId is null || a.Id == accountId).ToArray())
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
                    result = await _readUsage(account, timeout.Token);
                }
                catch (OperationCanceledException) { result = new(UsageState.Unavailable, "Usage request timed out. Automatic refresh will retry."); }
                catch (HttpRequestException) { result = new(UsageState.Unavailable, "Could not reach the provider. Check your internet connection."); }
                catch (JsonException) { result = new(UsageState.Unavailable, "The provider returned an unrecognized response. Check the CLI version and try again."); }
                catch (UnauthorizedAccessException) { result = new(UsageState.Unavailable, "The profile could not be read. Check its folder permissions."); }
                catch (Exception ex) when (ex is IOException or InvalidOperationException or System.ComponentModel.Win32Exception) { result = new(UsageState.Unavailable, "Could not read usage. Check the profile login and CLI installation, then refresh."); }
                if (!Settings.Accounts.Any(current => current.Id == account.Id && current.Provider == account.Provider && current.Directory == account.Directory)) continue;
                schedule.Complete(result, DateTimeOffset.UtcNow, Settings.RefreshMinutes);
                if (Results.TryGetValue(account.Id, out var previous))
                {
                    result = result with { Snapshot = result.Snapshot ?? previous.Snapshot };
                    var sameIdentity = result.AccountEmail is not null && result.AccountEmail == previous.AccountEmail;
                    if (!result.PlanFromProvider && previous.PlanFromProvider && result.State != UsageState.SignInRequired
                        && (result.State != UsageState.Ready || sameIdentity))
                        result = result with { PlanLabel = previous.PlanLabel, PlanFromProvider = true, AccountEmail = result.AccountEmail ?? previous.AccountEmail };
                }
                Results[account.Id] = result;
                ReadPlans();
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

    internal void ReportNotice(string message) { Notice = message; Changed?.Invoke(); }

    internal void CheckBillingReminders(DateOnly today)
    {
        if (IsDemo || IsRefreshing || BillingRemindersDue is null) return;
        var reminders = Settings.Accounts.Select(account => BillingSchedule.Reminder(account, today)).OfType<BillingReminder>().ToArray();
        if (reminders.Length == 0) return;
        var previous = Settings.Accounts.ToArray();
        foreach (var reminder in reminders)
        {
            var index = Settings.Accounts.FindIndex(account => account.Id == reminder.Account.Id);
            Settings.Accounts[index] = reminder.Account with
            {
                Billing = reminder.Account.Billing! with { LastReminderDate = reminder.Date, LastReminderDays = reminder.Threshold }
            };
        }
        try { Store.Save(Settings); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            Settings.Accounts = previous.ToList();
            Notice = "Renewal reminders could not be saved. Check local settings permissions.";
            Changed?.Invoke();
            return;
        }
        BillingRemindersDue.Invoke(reminders);
    }

    private void ReadPlans()
    {
        Plans.Clear();
        foreach (var account in Settings.Accounts)
            Plans[account.Id] = IsDemo ? account.Provider == ProviderKind.Claude ? "Pro" : "Plus" : Results.GetValueOrDefault(account.Id)?.PlanLabel ?? SubscriptionPlan.Read(account);
    }
    public void Dispose() { _timer.Stop(); _stop.Cancel(); _stop.Dispose(); _http.Dispose(); }

    private static HubSettings DemoSettings() => new()
    {
        Accounts = [new(Guid.NewGuid(), ProviderKind.Claude, "Personal", @"C:\Profiles\claude-personal", new(DateOnly.FromDateTime(DateTime.Now).AddDays(7))), new(Guid.NewGuid(), ProviderKind.Claude, "Second subscription", @"C:\Profiles\claude-second", new(DateOnly.FromDateTime(DateTime.Now).AddDays(3), AutoRenew: false)), new(Guid.NewGuid(), ProviderKind.Codex, "Personal", @"C:\Profiles\codex")],
        Projects = [new(Guid.NewGuid(), "AI SubHub", @"D:\Github\ai-subhub"), new(Guid.NewGuid(), "Website", @"D:\Demo\website"), new(Guid.NewGuid(), "Later", @"D:\Demo\later", true)]
    };
}
