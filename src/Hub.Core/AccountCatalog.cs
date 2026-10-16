namespace Hub.Core;

public static class AccountCatalog
{
    public static IReadOnlyList<AccountProfile> Ordered(HubSettings settings) => Enum.GetValues<ProviderKind>().SelectMany(provider => settings.Accounts.Where(a => a.Provider == provider)).ToArray();

    public static bool CanMove(HubSettings settings, Guid id, int direction) => Destination(settings, id, direction) is not null;

    public static bool Move(HubSettings settings, Guid id, int direction)
    {
        var destination = Destination(settings, id, direction);
        if (destination is null) return false;
        var index = settings.Accounts.FindIndex(a => a.Id == id);
        (settings.Accounts[index], settings.Accounts[destination.Value]) = (settings.Accounts[destination.Value], settings.Accounts[index]);
        return true;
    }

    private static int? Destination(HubSettings settings, Guid id, int direction)
    {
        if (direction is not (-1 or 1)) throw new ArgumentOutOfRangeException(nameof(direction));
        var index = settings.Accounts.FindIndex(a => a.Id == id);
        if (index < 0) return null;
        var provider = settings.Accounts[index].Provider;
        for (var candidate = index + direction; candidate >= 0 && candidate < settings.Accounts.Count; candidate += direction)
            if (settings.Accounts[candidate].Provider == provider) return candidate;
        return null;
    }
}
