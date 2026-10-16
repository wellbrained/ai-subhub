using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class UserDataPathsTests
{
    [Fact]
    public void RenameCopiesExistingMetadataWithoutRemovingOldSettingsOrOverwritingNewOnes()
    {
        var root = Path.Combine(Path.GetTempPath(), "subhub-migration-" + Guid.NewGuid());
        var legacy = Path.Combine(root, "AI Subscription Hub");
        Directory.CreateDirectory(legacy);
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Saved", @"C:\Profiles\saved");
        var settings = new HubSettings { Accounts = [account], Projects = [new(Guid.NewGuid(), "Hidden", @"D:\Projects\hidden", true)] };
        try
        {
            new SettingsStore(legacy).Save(settings);
            var current = UserDataPaths.Resolve(root);
            Assert.Equal(Path.Combine(root, "AI SubHub"), current);
            Assert.Equal(settings.Accounts, new SettingsStore(current).Load().Accounts);
            Assert.Equal(settings.Projects, new SettingsStore(current).Load().Projects);
            Assert.True(File.Exists(Path.Combine(legacy, "settings.json")));
            var updated = new HubSettings { Projects = settings.Projects };
            new SettingsStore(current).Save(updated);
            UserDataPaths.Resolve(root);
            Assert.Empty(new SettingsStore(current).Load().Accounts);
            Assert.Single(new SettingsStore(legacy).Load().Accounts);
        }
        finally { Directory.Delete(root, true); }
    }
}
