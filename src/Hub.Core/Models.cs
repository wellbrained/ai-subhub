using System.Text.Json;

namespace Hub.Core;

public enum ProviderKind { Claude, Codex }
public enum UsageState { Ready, Unavailable, SignInRequired, RateLimited, CliMissing }

public sealed record AccountProfile(Guid Id, ProviderKind Provider, string Label, string Directory);
public sealed record ProjectFolder(Guid Id, string Label, string Directory, bool IsHidden = false);
public sealed record QuotaWindow(string Label, double UsedPercent, DateTimeOffset? ResetsAt)
{
    public double RemainingPercent => Math.Clamp(100 - UsedPercent, 0, 100);
}
public sealed record UsageSnapshot(DateTimeOffset FetchedAt, IReadOnlyList<QuotaWindow> Windows);
public sealed record UsageResult(UsageState State, string Message, UsageSnapshot? Snapshot = null, TimeSpan? RetryAfter = null, string? AccountEmail = null, ResetAvailability? Resets = null, string? PlanLabel = null);
public sealed record ResetAvailability(int AvailableCount, DateTimeOffset? EarliestExpiry);
public sealed record CachedReading(Guid AccountId, UsageSnapshot Snapshot);

public sealed class HubSettings
{
    public int SchemaVersion { get; set; } = 1;
    public List<AccountProfile> Accounts { get; set; } = [];
    public List<ProjectFolder> Projects { get; set; } = [];
    public Guid? SelectedProjectId { get; set; }
    public string Theme { get; set; } = "Dark";
    public string UsageStyle { get; set; } = "Remaining";
    public int RefreshMinutes { get; set; } = 5;
    public bool StartWithWindows { get; set; }
    public string Terminal { get; set; } = "Windows Terminal";
    public string? ClaudeExecutable { get; set; }
    public string? CodexExecutable { get; set; }
    public List<CachedReading> Readings { get; set; } = [];
}

public static class ProfileRules
{
    public static string NormalizeDirectory(string path)
    {
        if (string.IsNullOrWhiteSpace(path)) throw new ArgumentException("Choose a profile folder.");
        return Path.TrimEndingDirectorySeparator(Path.GetFullPath(Environment.ExpandEnvironmentVariables(path.Trim())));
    }

    public static AccountProfile Create(IEnumerable<AccountProfile> accounts, ProviderKind provider, string label, string path, Guid? id = null)
    {
        label = label.Trim();
        if (label.Length is 0 or > 60) throw new ArgumentException("Use an account name from 1 to 60 characters.");
        path = NormalizeDirectory(path);
        if (accounts.Any(a => a.Id != id && a.Provider == provider && StringComparer.OrdinalIgnoreCase.Equals(a.Directory, path)))
            throw new ArgumentException("This profile is already connected. Rename its existing account instead.");
        return new(id ?? Guid.NewGuid(), provider, label, path);
    }
}

public static class QuotaParser
{
    public static string CompactLabel(string label)
    {
        var text = label.Replace(" allowance", "");
        if (text.StartsWith("codex · ", StringComparison.OrdinalIgnoreCase)) text = text[8..];
        return text switch { "Session" => "5-hour", "1-week" => "Weekly", _ => text };
    }
    public static IReadOnlyList<QuotaWindow> Claude(JsonElement root)
    {
        var windows = new List<QuotaWindow>();
        if (root.TryGetProperty("limits", out var limits) && limits.ValueKind == JsonValueKind.Array)
        {
            foreach (var limit in limits.EnumerateArray())
            {
                if (limit.ValueKind != JsonValueKind.Object || !Number(limit, "percent", out var percent)) continue;
                var kind = limit.TryGetProperty("kind", out var k) && k.ValueKind == JsonValueKind.String ? k.GetString() : null;
                var label = kind switch { "session" => "Session allowance", "weekly_all" => "Weekly allowance", "weekly_scoped" => "Weekly allowance", _ => kind ?? "Allowance" };
                if (limit.TryGetProperty("scope", out var scope) && scope.ValueKind == JsonValueKind.Object && scope.TryGetProperty("model", out var model) && model.ValueKind == JsonValueKind.Object && model.TryGetProperty("display_name", out var name) && name.ValueKind == JsonValueKind.String)
                    label += $" · {name.GetString()}";
                DateTimeOffset? reset = limit.TryGetProperty("resets_at", out var at) && at.ValueKind == JsonValueKind.String && DateTimeOffset.TryParse(at.GetString(), out var time) ? time : null;
                // is_active identifies the currently binding limit, not whether a window exists.
                windows.Add(new(label, percent, reset));
            }
            if (windows.Count > 0) return windows;
        }
        foreach (var (key, label) in new[] { ("five_hour", "5-hour allowance"), ("seven_day", "Weekly allowance"), ("seven_day_sonnet", "Weekly · Sonnet"), ("seven_day_opus", "Weekly · Opus") })
        {
            if (!root.TryGetProperty(key, out var bucket) || bucket.ValueKind != JsonValueKind.Object) continue;
            if (!Number(bucket, "utilization", out var used)) continue;
            DateTimeOffset? reset = bucket.TryGetProperty("resets_at", out var value) && value.ValueKind == JsonValueKind.String && DateTimeOffset.TryParse(value.GetString(), out var date) ? date : null;
            windows.Add(new(label, used, reset));
        }
        return windows;
    }

    public static IReadOnlyList<QuotaWindow> Codex(JsonElement root)
    {
        var windows = new List<QuotaWindow>();
        if (root.TryGetProperty("rateLimitsByLimitId", out var buckets) && buckets.ValueKind == JsonValueKind.Object)
        {
            foreach (var bucket in buckets.EnumerateObject()) AddCodex(windows, bucket.Value, bucket.Name);
        }
        if (windows.Count == 0 && root.TryGetProperty("rateLimits", out var fallback)) AddCodex(windows, fallback, "Codex");
        return windows;
    }

    private static void AddCodex(List<QuotaWindow> windows, JsonElement bucket, string key)
    {
        if (bucket.ValueKind != JsonValueKind.Object) return;
        var name = bucket.TryGetProperty("limitName", out var n) && n.ValueKind == JsonValueKind.String ? n.GetString() : null;
        foreach (var slot in new[] { "primary", "secondary" })
        {
            if (!bucket.TryGetProperty(slot, out var window) || window.ValueKind != JsonValueKind.Object || !Number(window, "usedPercent", out var used)) continue;
            var duration = window.TryGetProperty("windowDurationMins", out var minutes) && minutes.ValueKind == JsonValueKind.Number && minutes.TryGetInt64(out var m) ? m : 0;
            var label = duration switch { > 0 when duration % 10080 == 0 => $"{duration / 10080}-week allowance", > 0 when duration % 1440 == 0 => $"{duration / 1440}-day allowance", > 0 when duration % 60 == 0 => $"{duration / 60}-hour allowance", > 0 => $"{duration}-minute allowance", _ => slot };
            DateTimeOffset? reset = null;
            if (window.TryGetProperty("resetsAt", out var at) && at.ValueKind == JsonValueKind.Number && at.TryGetInt64(out var unix) && unix is >= -62135596800 and <= 253402300799) reset = DateTimeOffset.FromUnixTimeSeconds(unix);
            windows.Add(new($"{name ?? key} · {label}", used, reset));
        }
    }

    private static bool Number(JsonElement element, string key, out double number)
    {
        number = 0;
        return element.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.Number && value.TryGetDouble(out number) && double.IsFinite(number) && number is >= 0 and <= 100;
    }
}

public sealed class RefreshSchedule
{
    public int Failures { get; private set; }
    public DateTimeOffset NextAttempt { get; private set; }
    public void Complete(UsageResult result, DateTimeOffset now, int intervalMinutes)
    {
        Failures = result.State == UsageState.Ready ? 0 : Math.Min(Failures + 1, 6);
        var delay = TimeSpan.FromMinutes(Math.Clamp(intervalMinutes, 1, 60) * Math.Pow(2, Failures));
        if (delay > TimeSpan.FromHours(24)) delay = TimeSpan.FromHours(24);
        if (result.RetryAfter is { } retry && retry > delay) delay = retry;
        NextAttempt = now + delay;
    }
}
