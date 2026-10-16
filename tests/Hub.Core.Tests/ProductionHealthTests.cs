using System.Net;
using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class ProductionHealthTests
{
    [Fact]
    public async Task ProductionLivenessIsReadOnlyAndEndsWithAppLifetime()
    {
        using var http = new HttpClient { Timeout = TimeSpan.FromSeconds(3) };
        var health = new ProductionHealth(0);
        var address = $"http://127.0.0.1:{health.Port}";
        try
        {
            using var response = await http.GetAsync(address + "/api/health");
            Assert.Equal(HttpStatusCode.OK, response.StatusCode);
            Assert.Contains("AI SubHub", await response.Content.ReadAsStringAsync());
            using var unknown = await http.GetAsync(address + "/accounts");
            Assert.Equal(HttpStatusCode.NotFound, unknown.StatusCode);
            using var mutation = await http.PostAsync(address + "/api/health", null);
            Assert.Equal(HttpStatusCode.NotFound, mutation.StatusCode);
        }
        finally { health.Dispose(); }
        await Assert.ThrowsAsync<HttpRequestException>(() => http.GetAsync(address + "/api/health"));
    }
}
