using System.Text.Json;
using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class BillingScheduleTests
{
    [Theory]
    [InlineData("2026-01-31", "2026-02-01", "2026-02-28")]
    [InlineData("2028-01-31", "2028-02-01", "2028-02-29")]
    [InlineData("2026-01-31", "2026-03-01", "2026-03-31")]
    [InlineData("2026-01-31", "2026-04-01", "2026-04-30")]
    [InlineData("2026-12-31", "2027-01-01", "2027-01-31")]
    [InlineData("2026-10-24", "2026-10-24", "2026-10-24")]
    [InlineData("2026-10-24", "2026-10-23", "2026-10-24")]
    public void MonthlyDatesKeepOriginalDayWithoutDrift(string anchor, string today, string expected)
        => Assert.Equal(DateOnly.Parse(expected), BillingSchedule.NextDate(new(DateOnly.Parse(anchor)), DateOnly.Parse(today)));

    [Fact]
    public void CanceledAccessEndsWithoutRollingIntoAnotherMonth()
    {
        var billing = new MonthlyBilling(new(2026, 10, 24), AutoRenew: false);
        var today = new DateOnly(2026, 10, 25);
        Assert.Equal(billing.Date, BillingSchedule.NextDate(billing, today));
        Assert.StartsWith("Ended", BillingSchedule.Description(billing, today));
        Assert.Null(BillingSchedule.Reminder(Profile(billing), today));
    }

    [Theory]
    [InlineData(8, null)]
    [InlineData(7, 7)]
    [InlineData(5, 7)]
    [InlineData(3, 3)]
    [InlineData(1, 3)]
    [InlineData(0, 3)]
    public void CatchUpChoosesOnlyTheMostUrgentReminder(int days, int? threshold)
    {
        var today = new DateOnly(2026, 10, 24);
        var reminder = BillingSchedule.Reminder(Profile(new(today.AddDays(days))), today);
        Assert.Equal(threshold, reminder?.Threshold);
    }

    [Fact]
    public void RemindersAreDeduplicatedPerCycleAndThreshold()
    {
        var due = new DateOnly(2026, 10, 31);
        var billing = new MonthlyBilling(due, LastReminderDate: due, LastReminderDays: 7);
        Assert.Null(BillingSchedule.Reminder(Profile(billing), due.AddDays(-5)));
        Assert.Equal(3, BillingSchedule.Reminder(Profile(billing), due.AddDays(-3))!.Threshold);
        billing = billing with { LastReminderDays = 3 };
        Assert.Null(BillingSchedule.Reminder(Profile(billing), due));
        Assert.Equal(7, BillingSchedule.Reminder(Profile(billing), new(2026, 11, 23))!.Threshold);
    }

    [Fact]
    public void TrackingIsOptionalAndUnverifiedMonthlyProjectionsAreMarked()
    {
        var date = new DateOnly(2026, 10, 24);
        Assert.Null(BillingSchedule.Reminder(Profile(null), date));
        Assert.Null(BillingSchedule.Reminder(Profile(new(date, Reminders: false)), date));
        Assert.Contains("manual", BillingSchedule.Description(new(date), date));
        Assert.Contains("estimated", BillingSchedule.Description(new(date), date.AddDays(1)));
        Assert.StartsWith("Ends", BillingSchedule.Description(new(date, AutoRenew: false), date));
    }

    [Fact]
    public void SettingsRoundtripPreservesDatesAndReminderHistoryAndOldAccountsRemainCompatible()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-billing-" + Guid.NewGuid());
        try
        {
            var store = new SettingsStore(directory);
            var billing = new MonthlyBilling(new(2026, 10, 31), LastReminderDate: new(2026, 10, 31), LastReminderDays: 7);
            store.Save(new() { Accounts = [Profile(billing)] });
            Assert.Equal(billing, Assert.Single(store.Load().Accounts).Billing);
            var legacy = JsonSerializer.Deserialize<AccountProfile>("""{"Id":"00000000-0000-0000-0000-000000000001","Provider":0,"Label":"Synthetic","Directory":"synthetic"}""");
            Assert.Null(legacy!.Billing);
        }
        finally { Directory.Delete(directory, true); }
    }

    private static AccountProfile Profile(MonthlyBilling? billing) => new(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", Path.GetTempPath(), billing);
}
