using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class WindowPlacementTests
{
    [Fact]
    public void PlacementRoundtripPreservesExistingPreferencesAndAccounts()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-placement-store-" + Guid.NewGuid());
        try
        {
            var store = new SettingsStore(directory);
            var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", directory);
            var settings = new HubSettings { Accounts = [account], Theme = "Light", UsageStyle = "Used" };
            store.Save(settings);
            Assert.Null(store.Load().MainWindowPlacement);
            settings.MainWindowPlacement = new(-1200, 120, 760, 470, true);
            store.Save(settings);
            var loaded = store.Load();
            Assert.Equal(settings.MainWindowPlacement, loaded.MainWindowPlacement);
            Assert.Equal(account, Assert.Single(loaded.Accounts));
            Assert.Equal("Light", loaded.Theme);
            Assert.Equal("Used", loaded.UsageStyle);
        }
        finally { if (Directory.Exists(directory)) Directory.Delete(directory, true); }
    }
}
