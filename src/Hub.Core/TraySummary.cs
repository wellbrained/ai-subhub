using System.Globalization;

namespace Hub.Core;

public static class TraySummary
{
    public const int MaximumLength = 127;

    public static string Build(HubSettings settings, IReadOnlyDictionary<Guid, UsageResult> results, DateTimeOffset now)
    {
        var accounts = AccountCatalog.Ordered(settings);
        if (accounts.Count == 0) return "AI SubHub\nNo accounts connected";
        var lines = accounts.Select(account => AccountLine(account, results.GetValueOrDefault(account.Id), settings.RefreshMinutes, settings.UsageStyle, now)).ToArray();
        var text = settings.UsageStyle == "Used" ? "AI SubHub · highest used / reset" : "AI SubHub · lowest left / reset";
        for (var index = 0; index < lines.Length; index++)
        {
            var remaining = lines.Length - index - 1;
            var suffix = remaining > 0 ? $"\n+{remaining} more" : "";
            if (text.Length + 1 + lines[index].Length + suffix.Length > MaximumLength)
                return text + $"\n+{lines.Length - index} more";
            text += "\n" + lines[index];
        }
        return text;
    }

    private static string AccountLine(AccountProfile account, UsageResult? result, int interval, string style, DateTimeOffset now)
    {
        var label = Clip(new string(account.Label.Select(c => char.IsControl(c) ? ' ' : c).ToArray()).Trim(), 12);
        var prefix = $"{account.Provider} {label}: ";
        if (result is null) return prefix + "waiting";
        if (result.Snapshot is not { Windows.Count: > 0 } snapshot)
            return prefix + (result.State switch { UsageState.SignInRequired => "sign in", UsageState.CliMissing => "CLI missing", UsageState.RateLimited => "rate limited", _ => "unavailable" });
        var window = snapshot.Windows.MinBy(w => w.RemainingPercent)!;
        var stale = result.State != UsageState.Ready || now - snapshot.FetchedAt > TimeSpan.FromMinutes(Math.Clamp(interval, 1, 60) + 1);
        var reset = window.ResetsAt is not { } at ? "?" : at <= now ? "due" : Duration(at - now);
        return prefix + (stale ? "stale " : "") + UsageDisplay.Value(window, style).ToString("0.#", CultureInfo.InvariantCulture) + "% / " + reset;
    }

    private static string Duration(TimeSpan value) => value.TotalDays >= 1 ? $"{(int)value.TotalDays}d{value.Hours}h" : value.TotalHours >= 1 ? $"{(int)value.TotalHours}h{value.Minutes}m" : $"{Math.Max(1, (int)Math.Ceiling(value.TotalMinutes))}m";

    private static string Clip(string text, int length)
    {
        if (text.Length <= length) return text;
        var take = length - 1;
        if (char.IsHighSurrogate(text[take - 1])) take--;
        return text[..take] + "…";
    }
}
