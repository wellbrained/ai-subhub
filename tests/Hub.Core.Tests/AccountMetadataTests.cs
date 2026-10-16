using System.Text.Json;
using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class AccountMetadataTests
{
    [Theory]
    [InlineData("{}")]
    [InlineData("null")]
    [InlineData("{\"rateLimitResetCredits\":null}")]
    [InlineData("{\"rateLimitResetCredits\":{\"availableCount\":-1}}")]
    [InlineData("{\"rateLimitResetCredits\":{\"availableCount\":\"2\"}}")]
    public void MissingOrMalformedResetMetadataIsUnknownRatherThanZero(string json)
    {
        using var data = JsonDocument.Parse(json);
        Assert.Null(AccountMetadata.CodexResets(data.RootElement));
    }

    [Fact]
    public void ExplicitZeroAndCountWithoutDetailsAreKnown()
    {
        using var zero = JsonDocument.Parse("""{"rateLimitResetCredits":{"availableCount":0,"credits":[]}}""");
        Assert.Equal(0, AccountMetadata.CodexResets(zero.RootElement)!.AvailableCount);
        using var countOnly = JsonDocument.Parse("""{"rateLimitResetCredits":{"availableCount":3,"credits":null}}""");
        var result = AccountMetadata.CodexResets(countOnly.RootElement)!;
        Assert.Equal(3, result.AvailableCount); Assert.Null(result.EarliestExpiry);
    }

    [Fact]
    public void ResetCountRemainsAuthoritativeWhenDetailedListIsCapped()
    {
        using var data = JsonDocument.Parse("""{"rateLimitResetCredits":{"availableCount":4,"credits":[{"status":"redeemed","expiresAt":100},{"status":"available","expiresAt":2000},{"status":"available","expiresAt":1000},{"status":3,"expiresAt":1}]}}""");
        var result = AccountMetadata.CodexResets(data.RootElement)!;
        Assert.Equal(4, result.AvailableCount);
        Assert.Equal(DateTimeOffset.FromUnixTimeSeconds(1000), result.EarliestExpiry);
    }

    [Theory]
    [InlineData("{\"email\":\"user@example.test\"}", "user@example.test")]
    [InlineData("{\"email\":\"user@example.test\\nprivate\"}", null)]
    [InlineData("{\"email\":\"Display <user@example.test>\"}", null)]
    [InlineData("{\"email\":3}", null)]
    public void OnlyPlainProviderReportedEmailIsDisplayed(string json, string? expected)
    {
        using var data = JsonDocument.Parse(json);
        Assert.Equal(expected, AccountMetadata.Email(data.RootElement));
    }
}
