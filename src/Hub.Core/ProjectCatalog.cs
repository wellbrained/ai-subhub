namespace Hub.Core;

public static class ProjectCatalog
{
    public static IReadOnlyList<ProjectFolder> Visible(HubSettings settings) => settings.Projects.Where(p => !p.IsHidden).ToArray();

    public static bool Move(HubSettings settings, Guid id, int direction)
    {
        if (direction is not (-1 or 1)) throw new ArgumentOutOfRangeException(nameof(direction));
        var index = settings.Projects.FindIndex(p => p.Id == id);
        var destination = index + direction;
        if (index < 0 || destination < 0 || destination >= settings.Projects.Count) return false;
        (settings.Projects[index], settings.Projects[destination]) = (settings.Projects[destination], settings.Projects[index]);
        return true;
    }

    public static void SetHidden(HubSettings settings, Guid id, bool hidden)
    {
        var index = settings.Projects.FindIndex(p => p.Id == id);
        if (index >= 0) settings.Projects[index] = settings.Projects[index] with { IsHidden = hidden };
    }
}
