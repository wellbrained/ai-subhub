using System.Diagnostics;
using System.Net;
using System.Net.Http.Headers;
using System.Text.Json;

namespace Hub.Core;

public interface IUsageProvider
{
    Task<UsageResult> ReadAsync(AccountProfile account, CancellationToken cancellationToken);
}

public sealed class ClaudeUsageProvider(HttpClient http, LaunchService? launcher = null, Func<AccountProfile, CancellationToken, Task<JsonDocument?>>? credentialReader = null) : IUsageProvider
{
    public async Task<UsageResult> ReadAsync(AccountProfile account, CancellationToken cancellationToken)
    {
        var readCredentials = credentialReader ?? ClaudeCredentials.ReadAsync;
        using var credentials = await readCredentials(account, cancellationToken);
        if (credentials is null) return new(UsageState.SignInRequired, "Sign in with your Claude subscription to connect this profile.");
        if (!credentials.RootElement.TryGetProperty("claudeAiOauth", out var oauth) || !oauth.TryGetProperty("accessToken", out var token) || string.IsNullOrWhiteSpace(token.GetString()))
            return new(UsageState.SignInRequired, "No Claude subscription login found in this profile.");
        // Credentials stay in memory for this request. Never persist them or include response bodies in errors.
        using var request = new HttpRequestMessage(HttpMethod.Get, "https://api.anthropic.com/api/oauth/usage");
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token.GetString());
        request.Headers.Add("anthropic-beta", "oauth-2025-04-20");
        request.Headers.Add("anthropic-version", "2023-06-01");
        request.Headers.Add("x-app", "cli");
        request.Headers.UserAgent.ParseAdd("AI-SubHub/0.13.0");
        using var response = await http.SendAsync(request, cancellationToken);
        if (response.StatusCode is HttpStatusCode.Unauthorized or HttpStatusCode.Forbidden)
            return new(UsageState.SignInRequired, "Claude could not authorize usage access. Sign in again, then refresh.");
        if (response.StatusCode == HttpStatusCode.TooManyRequests)
        {
            var retry = response.Headers.RetryAfter?.Delta ?? (response.Headers.RetryAfter?.Date is { } date ? date - DateTimeOffset.UtcNow : TimeSpan.FromMinutes(10));
            return new(UsageState.RateLimited, "Claude asked us to slow down. Automatic refresh will retry later.", RetryAfter: retry);
        }
        if (!response.IsSuccessStatusCode) return new(UsageState.Unavailable, $"Claude usage is unavailable (HTTP {(int)response.StatusCode}). This unofficial endpoint may have changed.");
        using var data = JsonDocument.Parse(await response.Content.ReadAsStringAsync(cancellationToken));
        var windows = QuotaParser.Claude(data.RootElement);
        var email = await AccountMetadata.ClaudeEmailAsync(launcher, account, cancellationToken);
        return windows.Count == 0 ? new(UsageState.Unavailable, "Claude returned no recognized subscription allowances.") : new(UsageState.Ready, "Connected", new(DateTimeOffset.UtcNow, windows), AccountEmail: email, PlanLabel: SubscriptionPlan.Parse(ProviderKind.Claude, credentials.RootElement));
    }
}

public sealed class CodexUsageProvider(LaunchService launcher) : IUsageProvider
{
    public async Task<UsageResult> ReadAsync(AccountProfile account, CancellationToken cancellationToken)
    {
        var executable = launcher.Executable(ProviderKind.Codex);
        if (executable is null) return new(UsageState.CliMissing, "Codex CLI was not found. Set its executable in Settings → Tools.");
        if (!Directory.Exists(account.Directory)) return new(UsageState.SignInRequired, "Sign in with ChatGPT to connect this profile.");
        await using var rpc = new CodexRpc(executable, account);
        await rpc.CallAsync("initialize", new { clientInfo = new { name = "ai_subhub", title = "AI SubHub", version = "0.13.0" } }, cancellationToken);
        await rpc.NotifyAsync("initialized", new { }, cancellationToken);
        var identity = await rpc.CallAsync("account/read", new { refreshToken = false }, cancellationToken);
        if (!identity.TryGetProperty("account", out var signedIn) || signedIn.ValueKind != JsonValueKind.Object)
            return new(UsageState.SignInRequired, "Sign in with ChatGPT to connect this profile.");
        if (signedIn.TryGetProperty("type", out var type) && type.GetString() == "apiKey")
            return new(UsageState.Unavailable, "This profile uses an API key. Sign in with a ChatGPT subscription for subscription usage.");
        var data = await rpc.CallAsync("account/rateLimits/read", null, cancellationToken);
        var windows = QuotaParser.Codex(data);
        return windows.Count == 0 ? new(UsageState.Unavailable, "Codex returned no subscription allowances for this account.") : new(UsageState.Ready, "Connected", new(DateTimeOffset.UtcNow, windows), AccountEmail: AccountMetadata.Email(signedIn), Resets: AccountMetadata.CodexResets(data), PlanLabel: signedIn.TryGetProperty("planType", out var plan) && plan.ValueKind == JsonValueKind.String ? SubscriptionPlan.Label(plan.GetString()) : null);
    }
}

internal sealed class CodexRpc : IAsyncDisposable
{
    private readonly Process _process;
    private int _id;
    public CodexRpc(string executable, AccountProfile account)
    {
        var start = CliProcess.Create(executable, account, "app-server", "--listen", "stdio://");
        start.RedirectStandardInput = true; start.RedirectStandardOutput = true; start.RedirectStandardError = true;
        _process = Process.Start(start) ?? throw new IOException("Could not start Codex.");
        // Drain stderr without storing or logging potentially sensitive CLI diagnostics.
        _process.ErrorDataReceived += (_, _) => { };
        _process.BeginErrorReadLine();
    }

    public async Task<JsonElement> CallAsync(string method, object? parameters, CancellationToken ct)
    {
        var id = ++_id;
        await _process.StandardInput.WriteLineAsync(JsonSerializer.Serialize(new { id, method, @params = parameters }).AsMemory(), ct);
        await _process.StandardInput.FlushAsync(ct);
        while (await _process.StandardOutput.ReadLineAsync(ct) is { } line)
        {
            using var message = JsonDocument.Parse(line);
            var root = message.RootElement;
            if (!root.TryGetProperty("id", out var responseId) || responseId.ValueKind != JsonValueKind.Number || !responseId.TryGetInt32(out var value) || value != id) continue;
            if (root.TryGetProperty("error", out _)) throw new InvalidOperationException($"Codex could not complete {method}. Check its login and CLI version, then refresh.");
            if (root.TryGetProperty("result", out var result)) return result.Clone();
            throw new JsonException("Codex returned an unexpected response.");
        }
        throw new IOException("Codex closed its usage connection. Check the CLI installation and try again.");
    }

    public async Task NotifyAsync(string method, object parameters, CancellationToken ct)
    {
        await _process.StandardInput.WriteLineAsync(JsonSerializer.Serialize(new { method, @params = parameters }).AsMemory(), ct);
        await _process.StandardInput.FlushAsync(ct);
    }

    public ValueTask DisposeAsync()
    {
        if (!_process.HasExited) _process.Kill(entireProcessTree: true);
        _process.Dispose();
        return ValueTask.CompletedTask;
    }
}
