using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class AccountTests
{
    private static readonly DateTimeOffset Now = DateTimeOffset.Parse("2026-10-15T20:00:00Z");
    private static AccountProfile Account(string label, ProviderKind provider = ProviderKind.Claude) => new(Guid.NewGuid(), provider, label, @"C:\Profiles\" + label);

    [Fact]
    public void OrderingMovesWithinProviderAndPreservesProfilesAcrossReload()
    {
        var first = Account("First"); var codex = Account("Codex", ProviderKind.Codex); var second = Account("Second");
        var settings = new HubSettings { Accounts = [first, codex, second] };
        Assert.False(AccountCatalog.Move(settings, first.Id, -1));
        Assert.False(AccountCatalog.Move(settings, codex.Id, 1));
        Assert.True(AccountCatalog.Move(settings, second.Id, -1));
        Assert.Equal(new[] { second, codex, first }, settings.Accounts);
        Assert.Equal(new[] { second, first, codex }, AccountCatalog.Ordered(settings));
        var directory = Path.Combine(Path.GetTempPath(), "hub-account-tests-" + Guid.NewGuid());
        try
        {
            var store = new SettingsStore(directory); store.Save(settings);
            Assert.Equal(settings.Accounts, store.Load().Accounts);
        }
        finally { if (Directory.Exists(directory)) Directory.Delete(directory, true); }
    }

    [Fact]
    public void TooltipUsesResetFromLowestRemainingBucket()
    {
        var account = Account("Work");
        var result = new UsageResult(UsageState.Ready, "", new(Now, [new("Session", 10, Now.AddHours(1)), new("Weekly", 80, Now.AddDays(2))]));
        var text = TraySummary.Build(new() { Accounts = [account] }, new Dictionary<Guid, UsageResult> { [account.Id] = result }, Now);
        Assert.Contains("Claude Work: 20% / 2d0h", text);
    }

    [Theory]
    [InlineData(UsageState.Ready, 10)]
    [InlineData(UsageState.Unavailable, 0)]
    public void TooltipMarksOldOrFailedReadingsStaleAndNeverRefillsOverdueQuota(UsageState state, int age)
    {
        var account = Account("Work");
        var result = new UsageResult(state, "", new(Now.AddMinutes(-age), [new("Session", 100, Now.AddMinutes(-1))]));
        var text = TraySummary.Build(new() { Accounts = [account] }, new Dictionary<Guid, UsageResult> { [account.Id] = result }, Now);
        Assert.Contains("stale 0% / due", text);
    }

    [Fact]
    public void TooltipDoesNotInventUsageForMissingData()
    {
        var account = Account("Work");
        var text = TraySummary.Build(new() { Accounts = [account] }, new Dictionary<Guid, UsageResult> { [account.Id] = new(UsageState.SignInRequired, "private error") }, Now);
        Assert.Contains("sign in", text); Assert.DoesNotContain("%", text); Assert.DoesNotContain("private", text);
    }

    [Fact]
    public void TooltipBoundsWindowsTextAndKeepsOverflowCountWithoutBrokenLabels()
    {
        var settings = new HubSettings { Accounts = Enumerable.Range(0, 25).Select(i => Account("abcdefghij😀\n" + i)).ToList() };
        var text = TraySummary.Build(settings, new Dictionary<Guid, UsageResult>(), Now);
        Assert.InRange(text.Length, 1, TraySummary.MaximumLength);
        Assert.Contains("abcdefghij…: waiting", text);
        Assert.DoesNotContain('\r', text);
        Assert.Matches(@"\+\d+ more$", text);
        Assert.Equal(25, text.Split('\n').Count(line => line.Contains(": waiting")) + int.Parse(text.Split('\n')[^1].Split(' ')[0][1..]));
    }
}
