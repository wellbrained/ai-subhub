using System.Diagnostics;
using System.Net;
using System.Text.Json;
using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class UsageTests
{
    [Fact]
    public void ExplicitPowerShellTargetOverridesTerminalPreferenceAndKeepsAccountIsolation()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-launch-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var executable = Path.Combine(directory, "fake-cli.exe"); File.WriteAllText(executable, "");
            var settings = new HubSettings { Terminal = "Windows Terminal", ClaudeExecutable = executable };
            var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Personal", Path.Combine(directory, "profile"));
            var start = new LaunchService(settings).CreateStartInfo(account, directory, target: TerminalTarget.PowerShell);
            Assert.Equal(OperatingSystem.IsMacOS() ? "/usr/bin/osascript" : "powershell.exe", start.FileName);
            Assert.DoesNotContain("new-tab", start.ArgumentList);
            Assert.Equal(account.Directory, start.Environment["CLAUDE_CONFIG_DIR"]);
            Assert.False(Directory.Exists(account.Directory));
        }
        finally { Directory.Delete(directory, true); }
    }

    [Fact]
    public void ClaudeSelfDescribingLimitsIncludeInactiveAndModelScopedWindows()
    {
        using var json = JsonDocument.Parse("""{"five_hour":null,"limits":[{"kind":"session","percent":12.5,"is_active":false,"scope":null},{"kind":"weekly_all","percent":77,"is_active":false},{"kind":"weekly_scoped","percent":100,"is_active":true,"scope":{"model":{"display_name":"Model A"}}}]}""");
        var windows = QuotaParser.Claude(json.RootElement);
        Assert.Equal(3, windows.Count);
        Assert.Equal(87.5, windows[0].RemainingPercent);
        Assert.Equal(23, windows[1].RemainingPercent);
        Assert.Contains("Model A", windows[2].Label);
        Assert.Equal(0, windows[2].RemainingPercent);
    }

    [Fact]
    public void ServerCooldownLongerThanOneDayIsPreserved()
    {
        var schedule = new RefreshSchedule(); var now = DateTimeOffset.UtcNow;
        schedule.Complete(new(UsageState.RateLimited, "slow down", RetryAfter: TimeSpan.FromHours(48)), now, 5);
        Assert.Equal(now.AddHours(48), schedule.NextAttempt);
    }

    [Fact]
    public void ClaudeMissingAndNullAllowancesAreNotZero()
    {
        using var json = JsonDocument.Parse("""{"five_hour":null,"seven_day":{"utilization":27.5,"resets_at":"2026-10-20T10:00:00Z"}}""");
        var quota = Assert.Single(QuotaParser.Claude(json.RootElement));
        Assert.Equal(72.5, quota.RemainingPercent);
        Assert.NotNull(quota.ResetsAt);
    }

    [Fact]
    public void CodexUsesEveryBucketAndItsReturnedDuration()
    {
        using var json = JsonDocument.Parse("""{"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":20,"windowDurationMins":300,"resetsAt":1792000000},"secondary":null},"review":{"primary":{"usedPercent":45,"windowDurationMins":10080,"resetsAt":null}}},"rateLimits":{"primary":{"usedPercent":99}}}""");
        var quotas = QuotaParser.Codex(json.RootElement);
        Assert.Equal(2, quotas.Count);
        Assert.Contains("5-hour", quotas[0].Label);
        Assert.Contains("1-week", quotas[1].Label);
        Assert.Equal(80, quotas[0].RemainingPercent);
        Assert.Null(quotas[1].ResetsAt);
    }

    [Fact]
    public void InvalidOrMissingQuotaNumbersAreUnavailable()
    {
        using var json = JsonDocument.Parse("""{"rateLimits":{"primary":{"usedPercent":null},"secondary":{"usedPercent":-5}}}""");
        Assert.Empty(QuotaParser.Codex(json.RootElement));
    }

    [Fact]
    public void RateLimitBackoffHonorsRetryAfterAndSuccessResetsIt()
    {
        var schedule = new RefreshSchedule(); var now = DateTimeOffset.UtcNow;
        schedule.Complete(new(UsageState.RateLimited, "slow down", RetryAfter: TimeSpan.FromMinutes(25)), now, 5);
        Assert.Equal(now.AddMinutes(25), schedule.NextAttempt);
        schedule.Complete(new(UsageState.Ready, "ok"), now, 5);
        Assert.Equal(0, schedule.Failures);
        Assert.Equal(now.AddMinutes(5), schedule.NextAttempt);
    }

    [Fact]
    public void DuplicateProfileCannotPretendToBeAnotherAccountButCanBeRenamed()
    {
        var first = ProfileRules.Create([], ProviderKind.Claude, "First", Path.GetTempPath());
        Assert.Throws<ArgumentException>(() => ProfileRules.Create([first], ProviderKind.Claude, "Second", Path.GetTempPath()));
        var renamed = ProfileRules.Create([first], ProviderKind.Claude, "Renamed", Path.GetTempPath(), first.Id);
        Assert.Equal(first.Id, renamed.Id);
        Assert.Equal(first.Directory, renamed.Directory);
    }

    [Fact]
    public void LaunchProfileClearsBillingOverridesAndPreservesOtherEnvironment()
    {
        var start = new ProcessStartInfo("tool.exe");
        start.Environment["ANTHROPIC_API_KEY"] = "fake-test-key";
        start.Environment["OPENAI_API_KEY"] = "fake-test-key";
        start.Environment["TEST_SENTINEL"] = "keep";
        SubscriptionEnvironment.Apply(start, new(Guid.NewGuid(), ProviderKind.Claude, "A", @"C:\Profiles\A"));
        Assert.False(start.Environment.ContainsKey("ANTHROPIC_API_KEY"));
        Assert.False(start.Environment.ContainsKey("OPENAI_API_KEY"));
        Assert.Equal(@"C:\Profiles\A", start.Environment["CLAUDE_CONFIG_DIR"]);
        Assert.Equal("keep", start.Environment["TEST_SENTINEL"]);
        Assert.Equal("'a''b;$(x)'", SubscriptionEnvironment.PowerShellQuote("a'b;$(x)"));
    }

    [Fact]
    public void SettingsRoundTripContainsNoCredentialsAndPreservesUnreadableFile()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-tests-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var store = new SettingsStore(directory);
            var settings = new HubSettings { Accounts = [new(Guid.NewGuid(), ProviderKind.Codex, "Mine", @"C:\Profiles\mine")] };
            store.Save(settings);
            Assert.Equal("Mine", Assert.Single(store.Load().Accounts).Label);
            Assert.DoesNotContain("accessToken", File.ReadAllText(Path.Combine(directory, "settings.json")));
            File.WriteAllText(Path.Combine(directory, "settings.json"), "broken data");
            Assert.Empty(store.Load().Accounts);
            Assert.NotNull(store.LoadWarning);
            Assert.Equal("broken data", File.ReadAllText(Assert.Single(Directory.GetFiles(directory, "settings.json.unreadable-*"))));
        }
        finally { Directory.Delete(directory, true); }
    }

    [Fact]
    public async Task ClaudeUnauthorizedResponseDoesNotExposeServerBodyOrToken()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-tests-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            await File.WriteAllTextAsync(Path.Combine(directory, ".credentials.json"), """{"claudeAiOauth":{"accessToken":"fake-test-token"}}""");
            using var http = new HttpClient(new StubHandler(HttpStatusCode.Unauthorized, "sensitive fake provider body"));
            var result = await new ClaudeUsageProvider(http, credentialReader: async (_, ct) => JsonDocument.Parse(await File.ReadAllTextAsync(Path.Combine(directory, ".credentials.json"), ct))).ReadAsync(new(Guid.NewGuid(), ProviderKind.Claude, "Test", directory), CancellationToken.None);
            Assert.Equal(UsageState.SignInRequired, result.State);
            Assert.DoesNotContain("sensitive", result.Message);
            Assert.DoesNotContain("fake-test-token", result.Message);
        }
        finally { Directory.Delete(directory, true); }
    }

    private sealed class StubHandler(HttpStatusCode status, string body) : HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) => Task.FromResult(new HttpResponseMessage(status) { Content = new StringContent(body) });
    }
}
