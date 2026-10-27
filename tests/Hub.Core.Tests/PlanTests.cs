using System.Text;
using System.Text.Json;
using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class PlanTests
{
    [Theory]
    [InlineData("pro", "default_claude_ai", "Pro")]
    [InlineData("max", "default_claude_max_5x", "Max 5×")]
    [InlineData("max", "default_claude_max_20x", "Max 20×")]
    [InlineData("future-private-value", "unknown", "Plan unknown")]
    public void ClaudeUsesKnownMetadataWithoutExposingUnknownValues(string plan, string tier, string expected)
    {
        using var data = JsonDocument.Parse(JsonSerializer.Serialize(new { claudeAiOauth = new { subscriptionType = plan, rateLimitTier = tier } }));
        Assert.Equal(expected, SubscriptionPlan.Parse(ProviderKind.Claude, data.RootElement));
    }

    [Fact]
    public void CodexReadsOnlyKnownPlanClaimAndHandlesInvalidTokens()
    {
        var payload = Convert.ToBase64String(Encoding.UTF8.GetBytes("""{"https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}""")).TrimEnd('=').Replace('+', '-').Replace('/', '_');
        using var valid = JsonDocument.Parse(JsonSerializer.Serialize(new { tokens = new { id_token = "header." + payload + ".signature" } }));
        Assert.Equal("Plus", SubscriptionPlan.Parse(ProviderKind.Codex, valid.RootElement));
        using var invalid = JsonDocument.Parse("""{"tokens":{"id_token":"header.!.signature"}}""");
        Assert.Equal("Plan unknown", SubscriptionPlan.Parse(ProviderKind.Codex, invalid.RootElement));
        using var malformed = JsonDocument.Parse("""{"tokens":null} """);
        Assert.Equal("Plan unknown", SubscriptionPlan.Parse(ProviderKind.Codex, malformed.RootElement));
    }

    [Fact]
    public void MissingOrUnreadableProfileDoesNotBreakUsageUi()
    {
        Assert.Equal("Plan unknown", SubscriptionPlan.Read(new(Guid.NewGuid(), ProviderKind.Claude, "Test", Path.Combine(Path.GetTempPath(), Guid.NewGuid().ToString()))));
    }

    [Theory]
    [InlineData("null")]
    [InlineData("{}")]
    [InlineData("{\"organization\":null}")]
    [InlineData("{\"organization\":{\"organization_type\":\"private-value\",\"plan_display_name\":\"private\"}}")]
    public void UnrecognizedLiveProfileDoesNotLeakArbitraryLabels(string json)
    {
        using var data = JsonDocument.Parse(json);
        Assert.Null(SubscriptionPlan.ClaudeProfile(data.RootElement));
    }

    [Fact]
    public void FreshCodexRateLimitPlanWinsOverCachedAccountIdentity()
    {
        using var rateLimits = JsonDocument.Parse("""{"rateLimits":{"planType":"pro"}}""");
        using var account = JsonDocument.Parse("""{"planType":"plus"}""");
        Assert.Equal("Pro", SubscriptionPlan.CodexCurrent(rateLimits.RootElement, account.RootElement));
        using var missing = JsonDocument.Parse("null");
        Assert.Equal("Plus", SubscriptionPlan.CodexCurrent(missing.RootElement, account.RootElement));
    }
}
