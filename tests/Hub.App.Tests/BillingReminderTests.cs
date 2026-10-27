using Hub.Core;
using Xunit;

namespace Hub.App.Tests;

public sealed class BillingReminderTests
{
    [Fact]
    public void DeliveredReminderHistorySurvivesRestartAndStillAllowsThreeDayReminder()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-reminder-" + Guid.NewGuid());
        var today = new DateOnly(2026, 10, 24);
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", directory, new(today.AddDays(7)));
        var store = new SettingsStore(directory); store.Save(new() { Accounts = [account] });
        try
        {
            using (var hub = new HubController(false, store, null, null))
            {
                var delivered = new List<BillingReminder>();
                hub.BillingRemindersDue += reminders => delivered.AddRange(reminders);
                hub.CheckBillingReminders(today); hub.CheckBillingReminders(today);
                Assert.Equal(7, Assert.Single(delivered).Threshold);
            }
            using (var restarted = new HubController(false, store, null, null))
            {
                var delivered = new List<BillingReminder>();
                restarted.BillingRemindersDue += reminders => delivered.AddRange(reminders);
                restarted.CheckBillingReminders(today); Assert.Empty(delivered);
                restarted.CheckBillingReminders(today.AddDays(4)); restarted.CheckBillingReminders(today.AddDays(5));
                Assert.Equal(3, Assert.Single(delivered).Threshold);
                restarted.CheckBillingReminders(new(2026, 11, 23));
                Assert.Equal(2, delivered.Count);
                restarted.RemoveAccount(account.Id);
                restarted.CheckBillingReminders(new(2026, 11, 27));
                Assert.Equal(2, delivered.Count);
            }
        }
        finally { Directory.Delete(directory, true); }
    }

    [Fact]
    public void NoListenerDoesNotConsumeAReminder()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-reminder-" + Guid.NewGuid());
        var today = new DateOnly(2026, 10, 24);
        var store = new SettingsStore(directory);
        store.Save(new() { Accounts = [new(Guid.NewGuid(), ProviderKind.Codex, "Synthetic", directory, new(today.AddDays(3)))] });
        try
        {
            using var hub = new HubController(false, store, null, null);
            hub.CheckBillingReminders(today);
            Assert.Null(store.Load().Accounts[0].Billing!.LastReminderDate);
        }
        finally { Directory.Delete(directory, true); }
    }
}
