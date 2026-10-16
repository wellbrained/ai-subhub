using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class UsageDisplayTests
{
    [Fact]
    public void StyleComplementsPercentageAndFillWithoutChangingSourceData()
    {
        var window = new QuotaWindow("5-hour", 23, null);
        Assert.Equal(77, UsageDisplay.Value(window, "Remaining"));
        Assert.Equal("77% remaining", UsageDisplay.Text(window, "Remaining"));
        Assert.Equal(23, UsageDisplay.Value(window, "Used"));
        Assert.Equal("23% used", UsageDisplay.Text(window, "Used"));
        Assert.Equal(23, window.UsedPercent);
    }

    [Fact]
    public void TrayStyleKeepsMostConstrainedWindowAndItsOwnReset()
    {
        var now = DateTimeOffset.Parse("2026-10-16T12:00:00Z");
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Codex, "Test", "test");
        var result = new UsageResult(UsageState.Ready, "", new(now, [new("5-hour", 20, now.AddHours(1)), new("Weekly", 90, now.AddDays(3))]));
        var settings = new HubSettings { Accounts = [account], UsageStyle = "Used" };
        var text = TraySummary.Build(settings, new Dictionary<Guid, UsageResult> { [account.Id] = result }, now);
        Assert.Contains("highest used", text); Assert.Contains("90% / 3d0h", text);
    }

    [Fact]
    public void PreferencePersistsAndOlderOrUnknownValuesDefaultToRemaining()
    {
        var directory = Path.Combine(Path.GetTempPath(), "subhub-display-" + Guid.NewGuid());
        try
        {
            var store = new SettingsStore(directory); store.Save(new() { UsageStyle = "Used" });
            Assert.Equal("Used", store.Load().UsageStyle);
            File.WriteAllText(Path.Combine(directory, "settings.json"), "{}");
            Assert.Equal("Remaining", store.Load().UsageStyle);
            File.WriteAllText(Path.Combine(directory, "settings.json"), """{"UsageStyle":"other"}""");
            Assert.Equal("Remaining", store.Load().UsageStyle);
        }
        finally { Directory.Delete(directory, true); }
    }
}
