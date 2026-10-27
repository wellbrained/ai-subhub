using Hub.Core;
using Xunit;

namespace Hub.App.Tests;

public sealed class PlanRefreshTests
{
    [Theory]
    [InlineData(UsageState.SignInRequired, null)]
    [InlineData(UsageState.Ready, "other@example.test")]
    [InlineData(UsageState.Ready, null)]
    public async Task OldProviderPlanDoesNotFollowSignOutOrUnverifiedAccountSwitch(UsageState state, string? email)
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-plan-switch-" + Guid.NewGuid());
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", directory);
        var store = new SettingsStore(directory); store.Save(new() { Accounts = [account] });
        var reads = 0;
        try
        {
            using var hub = new HubController(false, store, (_, _) => Task.FromResult(++reads == 1
                ? new UsageResult(UsageState.Ready, "Connected", AccountEmail: "first@example.test", PlanLabel: "Max 5×", PlanFromProvider: true)
                : new UsageResult(state, "Synthetic", AccountEmail: email, PlanLabel: state == UsageState.Ready ? "Pro" : null)), null);
            await hub.RefreshAsync(true); await hub.RefreshAsync(true);
            Assert.NotEqual("Max 5×", hub.Plans[account.Id]);
            Assert.False(hub.Results[account.Id].PlanFromProvider);
        }
        finally { Directory.Delete(directory, true); }
    }

    [Fact]
    public async Task BillingEditDuringUsageReadDoesNotDropTheReading()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-plan-concurrent-" + Guid.NewGuid());
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", directory);
        var store = new SettingsStore(directory); store.Save(new() { Accounts = [account] });
        var reading = new TaskCompletionSource<UsageResult>(TaskCreationOptions.RunContinuationsAsynchronously);
        try
        {
            using var hub = new HubController(false, store, (_, _) => reading.Task, null);
            var refresh = hub.RefreshAsync(true);
            hub.Settings.Accounts[0] = account with { Billing = new(new(2026, 11, 24)) };
            reading.SetResult(new(UsageState.Ready, "Connected", PlanLabel: "Max 5×", PlanFromProvider: true));
            await refresh;
            Assert.Equal("Max 5×", hub.Plans[account.Id]);
            Assert.Equal(new DateOnly(2026, 11, 24), hub.Settings.Accounts[0].Billing!.Date);
        }
        finally { Directory.Delete(directory, true); }
    }

    [Fact]
    public async Task ProviderPlanUpdatesBeforeChangeEventAndSurvivesOptionalMetadataFailure()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-plan-" + Guid.NewGuid());
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", directory);
        var store = new SettingsStore(directory); store.Save(new() { Accounts = [account] });
        var reads = 0;
        try
        {
            using var hub = new HubController(false, store, (_, _) => Task.FromResult(++reads switch
            {
                1 => new UsageResult(UsageState.Ready, "Connected", AccountEmail: "same@example.test", PlanLabel: "Max 5×", PlanFromProvider: true),
                2 => new UsageResult(UsageState.Ready, "Connected", AccountEmail: "same@example.test", PlanLabel: "Pro"),
                3 => new UsageResult(UsageState.Ready, "Connected", AccountEmail: "same@example.test", PlanLabel: "Pro", PlanFromProvider: true),
                _ => new UsageResult(UsageState.Unavailable, "Synthetic offline")
            }), null);
            hub.Changed += () =>
            {
                if (hub.Results.GetValueOrDefault(account.Id)?.PlanLabel is { } plan) Assert.Equal(plan, hub.Plans[account.Id]);
            };
            await hub.RefreshAsync(true); Assert.Equal("Max 5×", hub.Plans[account.Id]);
            await hub.RefreshAsync(true); Assert.Equal("Max 5×", hub.Plans[account.Id]);
            await hub.RefreshAsync(true); Assert.Equal("Pro", hub.Plans[account.Id]);
            await hub.RefreshAsync(true); Assert.Equal("Pro", hub.Plans[account.Id]);
            hub.ForgetReading(account.Id); hub.Save(); Assert.Equal("Plan unknown", hub.Plans[account.Id]);
        }
        finally { Directory.Delete(directory, true); }
    }
}
