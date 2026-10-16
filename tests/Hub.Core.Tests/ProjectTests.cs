using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class ProjectTests
{
    [Fact]
    public void HidingOnlyFiltersLaunchListAndShowingRestoresManualPosition()
    {
        var first = new ProjectFolder(Guid.NewGuid(), "First", @"C:\Projects\first");
        var second = new ProjectFolder(Guid.NewGuid(), "Second", @"C:\Projects\second");
        var settings = new HubSettings { Projects = [first, second] };
        ProjectCatalog.SetHidden(settings, first.Id, true);
        Assert.Equal(second.Id, Assert.Single(ProjectCatalog.Visible(settings)).Id);
        Assert.Equal(2, settings.Projects.Count);
        Assert.Equal(first.Directory, settings.Projects[0].Directory);
        ProjectCatalog.SetHidden(settings, first.Id, false);
        Assert.Equal(new[] { first.Id, second.Id }, ProjectCatalog.Visible(settings).Select(p => p.Id));
    }

    [Fact]
    public void ManualOrderAndHiddenStateSurviveSettingsReload()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-project-tests-" + Guid.NewGuid());
        var first = new ProjectFolder(Guid.NewGuid(), "First", @"C:\Projects\first");
        var second = new ProjectFolder(Guid.NewGuid(), "Second", @"C:\Projects\second", true);
        var third = new ProjectFolder(Guid.NewGuid(), "Third", @"C:\Projects\third");
        var settings = new HubSettings { Projects = [first, second, third] };
        Assert.False(ProjectCatalog.Move(settings, first.Id, -1));
        Assert.True(ProjectCatalog.Move(settings, third.Id, -1));
        Assert.True(ProjectCatalog.Move(settings, third.Id, -1));
        try
        {
            var store = new SettingsStore(directory); store.Save(settings);
            var loaded = store.Load();
            Assert.Equal(new[] { third.Id, first.Id, second.Id }, loaded.Projects.Select(p => p.Id));
            Assert.True(loaded.Projects[2].IsHidden);
            Assert.Equal(new[] { third.Id, first.Id }, ProjectCatalog.Visible(loaded).Select(p => p.Id));
        }
        finally { if (Directory.Exists(directory)) Directory.Delete(directory, true); }
    }

    [Fact]
    public void ExistingSettingsWithoutVisibilityFieldKeepTheirProjectsVisible()
    {
        var directory = Path.Combine(Path.GetTempPath(), "hub-project-tests-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            File.WriteAllText(Path.Combine(directory, "settings.json"), """{"SchemaVersion":1,"Projects":[{"Id":"11111111-1111-1111-1111-111111111111","Label":"Existing","Directory":"C:\\Projects\\existing"}]}""");
            var settings = new SettingsStore(directory).Load();
            Assert.False(Assert.Single(settings.Projects).IsHidden);
            Assert.Equal("Existing", Assert.Single(ProjectCatalog.Visible(settings)).Label);
        }
        finally { Directory.Delete(directory, true); }
    }

    [Fact]
    public void DesktopFolderLinkTreatsFolderNameAsDataRatherThanQueryParameters()
    {
        var path = Path.Combine(Path.GetTempPath(), "folder & q=do-stuff #one");
        var uri = LaunchService.ClaudeDesktopFolderUri(path);
        Assert.Equal("claude", uri.Scheme); Assert.Equal("code", uri.Host); Assert.Equal("/new", uri.AbsolutePath);
        Assert.Equal(Path.GetFullPath(path), Uri.UnescapeDataString(uri.Query["?folder=".Length..]));
        Assert.DoesNotContain("&q=", uri.Query);
    }
}
