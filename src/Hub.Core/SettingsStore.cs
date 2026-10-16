using System.Text.Json;
using System.Text.Json.Serialization;

namespace Hub.Core;

public sealed class SettingsStore(string directory)
{
    private readonly string _file = Path.Combine(directory, "settings.json");
    private static readonly JsonSerializerOptions JsonOptions = new() { WriteIndented = true, Converters = { new JsonStringEnumConverter() } };
    public string? LoadWarning { get; private set; }
    public string DirectoryPath => directory;

    public HubSettings Load()
    {
        if (!File.Exists(_file)) return new();
        try
        {
            var settings = JsonSerializer.Deserialize<HubSettings>(File.ReadAllText(_file), JsonOptions) ?? throw new JsonException();
            if (settings.SchemaVersion != 1) throw new JsonException();
            settings.Accounts ??= [];
            settings.Projects ??= [];
            settings.Readings ??= [];
            settings.RefreshMinutes = Math.Clamp(settings.RefreshMinutes, 1, 60);
            settings.Theme = settings.Theme is "Light" or "System" ? settings.Theme : "Dark";
            settings.UsageStyle = settings.UsageStyle == "Used" ? "Used" : "Remaining";
            return settings;
        }
        catch (Exception ex) when (ex is JsonException or IOException or UnauthorizedAccessException)
        {
            // Preserve unreadable settings rather than overwriting the only copy.
            var backup = _file + $".unreadable-{DateTime.UtcNow:yyyyMMddHHmmssfff}";
            File.Copy(_file, backup, false);
            LoadWarning = "Settings could not be read. A backup was preserved; starting with an empty configuration.";
            return new();
        }
    }

    public void Save(HubSettings settings)
    {
        Directory.CreateDirectory(directory);
        var temporary = _file + ".tmp";
        File.WriteAllText(temporary, JsonSerializer.Serialize(settings, JsonOptions));
        File.Move(temporary, _file, true);
    }
}
