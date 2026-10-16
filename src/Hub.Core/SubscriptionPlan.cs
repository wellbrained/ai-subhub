using System.Text;
using System.Text.Json;

namespace Hub.Core;

public static class SubscriptionPlan
{
    public static string Read(AccountProfile account)
    {
        try
        {
            var file = Path.Combine(account.Directory, account.Provider == ProviderKind.Claude ? ".credentials.json" : "auth.json");
            using var data = JsonDocument.Parse(File.ReadAllText(file));
            return Parse(account.Provider, data.RootElement);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException or FormatException or ArgumentException)
        { return "Plan unknown"; }
    }

    public static string Parse(ProviderKind provider, JsonElement root)
    {
        if (provider == ProviderKind.Claude)
        {
            if (!Object(root, "claudeAiOauth", out var oauth)) return "Plan unknown";
            var plan = Text(oauth, "subscriptionType");
            if (plan == "max") return Text(oauth, "rateLimitTier") switch
            { "default_claude_max_5x" => "Max 5×", "default_claude_max_20x" => "Max 20×", _ => "Max" };
            return Label(plan);
        }
        if (Text(root, "auth_mode") == "apikey") return "API key";
        if (!Object(root, "tokens", out var tokens) || Text(tokens, "id_token") is not { } token) return "Plan unknown";
        var parts = token.Split('.');
        if (parts.Length != 3) return "Plan unknown";
        try
        {
            var encoded = parts[1].Replace('-', '+').Replace('_', '/');
            encoded = encoded.PadRight(encoded.Length + (4 - encoded.Length % 4) % 4, '=');
            // Local display metadata only: this does not validate or authenticate the token.
            using var claims = JsonDocument.Parse(Encoding.UTF8.GetString(Convert.FromBase64String(encoded)));
            return Object(claims.RootElement, "https://api.openai.com/auth", out var auth) ? Label(Text(auth, "chatgpt_plan_type")) : "Plan unknown";
        }
        catch (Exception ex) when (ex is FormatException or JsonException) { return "Plan unknown"; }
    }

    private static string Label(string? value) => value?.ToLowerInvariant() switch
    { "free" => "Free", "pro" => "Pro", "plus" => "Plus", "max" => "Max", "team" => "Team", "business" => "Business", "enterprise" => "Enterprise", "edu" => "Edu", "go" => "Go", _ => "Plan unknown" };
    private static string? Text(JsonElement root, string key) => root.ValueKind == JsonValueKind.Object && root.TryGetProperty(key, out var value) && value.ValueKind == JsonValueKind.String ? value.GetString() : null;
    private static bool Object(JsonElement root, string key, out JsonElement value)
    { value = default; return root.ValueKind == JsonValueKind.Object && root.TryGetProperty(key, out value) && value.ValueKind == JsonValueKind.Object; }
}
