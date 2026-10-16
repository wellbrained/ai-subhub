using System.Diagnostics;
using System.Net.Mail;
using System.Text.Json;

namespace Hub.Core;

public static class AccountMetadata
{
    public static string? Email(JsonElement root, string key = "email")
    {
        if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty(key, out var field) || field.ValueKind != JsonValueKind.String) return null;
        var text = field.GetString();
        return text is { Length: <= 254 } && !text.Any(char.IsControl) && MailAddress.TryCreate(text, out var email) && email.Address == text ? text : null;
    }

    public static ResetAvailability? CodexResets(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty("rateLimitResetCredits", out var reset) || reset.ValueKind != JsonValueKind.Object
            || !reset.TryGetProperty("availableCount", out var count) || count.ValueKind != JsonValueKind.Number || !count.TryGetInt32(out var available) || available < 0) return null;
        DateTimeOffset? expiry = null;
        if (reset.TryGetProperty("credits", out var credits) && credits.ValueKind == JsonValueKind.Array)
            foreach (var credit in credits.EnumerateArray())
            {
                if (credit.ValueKind != JsonValueKind.Object || !credit.TryGetProperty("status", out var status) || status.ValueKind != JsonValueKind.String || status.GetString() != "available"
                    || !credit.TryGetProperty("expiresAt", out var at) || at.ValueKind != JsonValueKind.Number || !at.TryGetInt64(out var unix)
                    || unix is < -62135596800 or > 253402300799) continue;
                var date = DateTimeOffset.FromUnixTimeSeconds(unix);
                if (expiry is null || date < expiry) expiry = date;
            }
        return new(available, expiry);
    }

    public static async Task<string?> ClaudeEmailAsync(LaunchService? launcher, AccountProfile account, CancellationToken cancellationToken)
    {
        var executable = launcher?.Executable(ProviderKind.Claude);
        if (executable is null) return null;
        try
        {
            var start = CliProcess.Create(executable, account, "auth", "status");
            start.RedirectStandardOutput = true; start.RedirectStandardError = true;
            using var process = Process.Start(start);
            if (process is null) return null;
            process.ErrorDataReceived += (_, _) => { }; process.BeginErrorReadLine();
            using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken); timeout.CancelAfter(TimeSpan.FromSeconds(5));
            try
            {
                var output = await process.StandardOutput.ReadToEndAsync(timeout.Token);
                await process.WaitForExitAsync(timeout.Token);
                if (process.ExitCode != 0) return null;
                using var data = JsonDocument.Parse(output);
                return data.RootElement.TryGetProperty("loggedIn", out var signedIn) && signedIn.ValueKind == JsonValueKind.True ? Email(data.RootElement) : null;
            }
            finally { if (!process.HasExited) process.Kill(entireProcessTree: true); }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException or InvalidOperationException or OperationCanceledException or System.ComponentModel.Win32Exception) { return null; }
    }
}
