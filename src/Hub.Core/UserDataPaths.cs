namespace Hub.Core;

public static class UserDataPaths
{
    public static string Resolve(string localAppData)
    {
        var current = Path.Combine(localAppData, "AI SubHub");
        var target = Path.Combine(current, "settings.json");
        // Migration-only reference: preserve settings from the original product name.
        var legacy = Path.Combine(localAppData, "AI Subscription Hub", "settings.json");
        if (!File.Exists(target) && File.Exists(legacy))
        {
            Directory.CreateDirectory(current);
            File.Copy(legacy, target, overwrite: false);
        }
        return current;
    }
}
