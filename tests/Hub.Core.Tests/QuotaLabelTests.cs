using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class QuotaLabelTests
{
    [Theory]
    [InlineData("Session allowance", "5-hour")]
    [InlineData("5-hour allowance", "5-hour")]
    [InlineData("Weekly allowance", "Weekly")]
    [InlineData("codex · 5-hour allowance", "5-hour")]
    [InlineData("codex · 1-week allowance", "Weekly")]
    [InlineData("Weekly allowance · Sonnet", "Weekly · Sonnet")]
    [InlineData("Other · 3-day allowance", "Other · 3-day")]
    public void StandardWindowsLookConsistentWhileSpecificBucketsStayDistinct(string source, string expected)
        => Assert.Equal(expected, QuotaParser.CompactLabel(source));
}
