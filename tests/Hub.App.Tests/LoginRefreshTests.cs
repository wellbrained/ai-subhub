using Hub.Core;
using Xunit;

namespace Hub.App.Tests;

public sealed class LoginRefreshTests
{
    [Fact]
    public async Task WindowsTerminalLoginRefreshesUsageThroughRealCompletionHandoff()
    {
        if (!OperatingSystem.IsWindows()) return;
        var directory = Path.Combine(Path.GetTempPath(), "hub's login test " + Guid.NewGuid());
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", directory);
        var cli = Path.Combine(directory, "fake-claude.cmd");
        var authenticated = Path.Combine(directory, "synthetic-authenticated");
        var store = new SettingsStore(directory);
        var settings = new HubSettings { Accounts = [account], ClaudeExecutable = cli };
        store.Save(settings);
        await File.WriteAllTextAsync(cli, "@echo off\r\necho synthetic>\"" + authenticated + "\"\r\nexit /b 0\r\n");
        var refreshed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var reads = 0;
        System.Diagnostics.Process? process = null;
        try
        {
            using var hub = new HubController(false, store, (_, _) =>
            {
                reads++;
                return Task.FromResult(File.Exists(authenticated)
                    ? new UsageResult(UsageState.Ready, "Connected", new(DateTimeOffset.UtcNow, [new("5-hour", 20, null)]))
                    : new UsageResult(UsageState.SignInRequired, "Synthetic missing login"));
            }, (profile, completion) =>
            {
                var start = new LaunchService(settings).CreateStartInfo(profile, directory, login: true, target: TerminalTarget.PowerShell, completionFile: completion);
                start.ArgumentList.Remove("-NoExit");
                start.CreateNoWindow = true; start.RedirectStandardOutput = true; start.RedirectStandardError = true;
                process = System.Diagnostics.Process.Start(start)!;
                process.OutputDataReceived += (_, _) => { }; process.BeginOutputReadLine();
                process.ErrorDataReceived += (_, _) => { }; process.BeginErrorReadLine();
            });
            hub.Changed += () => { if (hub.Results.GetValueOrDefault(account.Id)?.State == UsageState.Ready) refreshed.TrySetResult(); };
            await hub.RefreshAsync(true);
            Assert.Equal(UsageState.SignInRequired, hub.Results[account.Id].State);
            hub.SignIn(account);
            await refreshed.Task.WaitAsync(TimeSpan.FromSeconds(10));
            await process!.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(5));
            Assert.Equal(2, reads);
            Assert.NotNull(hub.Results[account.Id].Snapshot);
        }
        finally { process?.Dispose(); Directory.Delete(directory, true); }
    }

    [Fact]
    public async Task LoginSuccessKeepsKnownProviderCooldown()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-login-cooldown-" + Guid.NewGuid());
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", directory);
        var store = new SettingsStore(directory); store.Save(new() { Accounts = [account] });
        var reads = 0;
        var completed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        try
        {
            using var hub = new HubController(false, store, (_, _) =>
            {
                reads++;
                return Task.FromResult(new UsageResult(UsageState.RateLimited, "Synthetic cooldown", RetryAfter: TimeSpan.FromHours(1)));
            }, (_, completion) => File.WriteAllText(completion, "0"));
            await hub.RefreshAsync(true);
            hub.Changed += () => { if (hub.Notice is null && !hub.IsRefreshing) completed.TrySetResult(); };
            hub.SignIn(account);
            await completed.Task.WaitAsync(TimeSpan.FromSeconds(5));
            Assert.Equal(1, reads);
            Assert.Equal(UsageState.RateLimited, hub.Results[account.Id].State);
        }
        finally { Directory.Delete(directory, true); }
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task LoginSuccessRefreshesOnlyItsAccountPastFailureBackoffAndAfterAnyActiveRead(bool finishDuringRead)
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-controller-login-" + Guid.NewGuid());
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", directory);
        var other = new AccountProfile(Guid.NewGuid(), ProviderKind.Codex, "Other", directory);
        var store = new SettingsStore(directory);
        store.Save(new() { Accounts = [account, other] });
        var initialStarted = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var initialMayFinish = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var updated = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var signedIn = false;
        var selectedReads = 0; var otherReads = 0;
        try
        {
            using var hub = new HubController(false, store, async (profile, ct) =>
            {
                if (profile.Id == other.Id) { otherReads++; return new(UsageState.Ready, "Synthetic other"); }
                selectedReads++;
                if (selectedReads == 1)
                {
                    initialStarted.SetResult();
                    if (finishDuringRead) await initialMayFinish.Task.WaitAsync(ct);
                    return new(UsageState.SignInRequired, "Synthetic missing login");
                }
                Assert.True(signedIn);
                return new(UsageState.Ready, "Connected", new(DateTimeOffset.UtcNow, [new("5-hour", 20, null)]), PlanLabel: "Pro");
            }, (_, completion) => { signedIn = true; File.WriteAllText(completion, "0"); });
            hub.Changed += () =>
            {
                if (hub.Results.GetValueOrDefault(account.Id)?.State == UsageState.Ready && hub.Plans.GetValueOrDefault(account.Id) == "Pro") updated.TrySetResult();
            };
            var initial = hub.RefreshAsync(true);
            await initialStarted.Task.WaitAsync(TimeSpan.FromSeconds(5));
            if (!finishDuringRead) await initial;
            hub.SignIn(account);
            initialMayFinish.TrySetResult();
            await initial;
            await updated.Task.WaitAsync(TimeSpan.FromSeconds(5));
            Assert.Equal(2, selectedReads);
            Assert.Equal(1, otherReads);
            Assert.NotNull(hub.Results[account.Id].Snapshot);
            Assert.Equal("Pro", hub.Plans[account.Id]);
            Assert.Null(hub.Notice);
        }
        finally { Directory.Delete(directory, true); }
    }

    [Fact]
    public async Task FailedLoginDoesNotRefreshUsage()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-controller-failed-login-" + Guid.NewGuid());
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", directory);
        var store = new SettingsStore(directory);
        store.Save(new() { Accounts = [account] });
        var reads = 0;
        var failed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        try
        {
            using var hub = new HubController(false, store, (_, _) => { reads++; return Task.FromResult(new UsageResult(UsageState.Ready, "")); },
                (_, completion) => File.WriteAllText(completion, "7"));
            hub.Changed += () => { if (hub.Notice?.Contains("did not complete successfully") == true) failed.TrySetResult(); };
            hub.SignIn(account);
            await failed.Task.WaitAsync(TimeSpan.FromSeconds(5));
            Assert.Equal(0, reads);
        }
        finally { Directory.Delete(directory, true); }
    }
}
