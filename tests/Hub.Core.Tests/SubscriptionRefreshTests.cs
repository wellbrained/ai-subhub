using System.Net;
using System.Text.Json;
using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class SubscriptionRefreshTests
{
    [Fact]
    public async Task LiveClaudePlanOverridesOldLoginMetadataWithoutChangingQuotas()
    {
        using var http = new HttpClient(new ProfileHandler(HttpStatusCode.OK));
        var result = await Read(http);
        Assert.Equal(UsageState.Ready, result.State);
        Assert.Equal("Max 5×", result.PlanLabel);
        Assert.Equal(75, Assert.Single(result.Snapshot!.Windows).RemainingPercent);
    }

    [Theory]
    [InlineData(HttpStatusCode.Forbidden)]
    [InlineData(HttpStatusCode.TooManyRequests)]
    public async Task OptionalProfileFailureKeepsUsageAndSavedPlan(HttpStatusCode status)
    {
        using var http = new HttpClient(new ProfileHandler(status));
        var result = await Read(http);
        Assert.Equal(UsageState.Ready, result.State);
        Assert.Equal("Pro", result.PlanLabel);
        Assert.DoesNotContain("private", result.Message);
    }

    [Theory]
    [InlineData("not-json")]
    [InlineData("null")]
    [InlineData("{\"organization\":{\"organization_type\":\"future-private-plan\"}}")]
    public async Task UnsupportedProfileDoesNotFailValidUsage(string body)
    {
        using var http = new HttpClient(new ProfileHandler(HttpStatusCode.OK, body));
        var result = await Read(http);
        Assert.Equal(UsageState.Ready, result.State); Assert.Equal("Pro", result.PlanLabel);
    }

    [Fact]
    public async Task OversizedProfileIsIgnoredWithoutBreakingUsage()
    {
        using var http = new HttpClient(new ProfileHandler(HttpStatusCode.OK, new string('x', 65537)));
        var result = await Read(http);
        Assert.Equal(UsageState.Ready, result.State); Assert.Equal("Pro", result.PlanLabel);
    }

    private static Task<UsageResult> Read(HttpClient http) => new ClaudeUsageProvider(http,
        credentialReader: (_, _) => Task.FromResult<JsonDocument?>(JsonDocument.Parse("""{"claudeAiOauth":{"accessToken":"synthetic-token","subscriptionType":"pro"}}""")))
        .ReadAsync(new(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", Path.GetTempPath()), CancellationToken.None);

    private sealed class ProfileHandler(HttpStatusCode profileStatus, string? profileBody = null) : HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
        {
            var profile = request.RequestUri!.AbsolutePath.EndsWith("/profile");
            return Task.FromResult(new HttpResponseMessage(profile ? profileStatus : HttpStatusCode.OK)
            {
                Content = new StringContent(profile
                    ? profileBody ?? """{"organization":{"organization_type":"claude_max","rate_limit_tier":"default_claude_max_5x"},"private":"not for display"}"""
                    : """{"five_hour":{"utilization":25,"resets_at":null}}""")
            });
        }
    }
}
