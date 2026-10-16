using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.Platform.Storage;
using Hub.Core;

namespace Hub.App;

internal static class Dialogs
{
    public static async Task<string?> Folder(Window owner)
    {
        var folders = await owner.StorageProvider.OpenFolderPickerAsync(new FolderPickerOpenOptions { Title = "Choose a folder", AllowMultiple = false });
        return folders.FirstOrDefault()?.TryGetLocalPath();
    }

    internal static Window AccountWindow(HubController hub, ProviderKind provider, AccountProfile? existing = null)
    {
        var dialog = Base(existing is null ? $"Add {provider} account" : "Edit account");
        var content = Ui.Stack(10);
        var fields = new Grid { ColumnDefinitions = new ColumnDefinitions("66,*"), RowDefinitions = new RowDefinitions("Auto,Auto"), RowSpacing = 8, ColumnSpacing = 8 };
        var name = Input(existing?.Label, "e.g. Personal or Work"); name.MaxLength = 60;
        fields.Children.Add(Ui.Text("Name", 12)); Grid.SetColumn(name, 1); fields.Children.Add(name);
        var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        var path = Input(existing?.Directory ?? Environment.GetEnvironmentVariable(provider == ProviderKind.Claude ? "CLAUDE_CONFIG_DIR" : "CODEX_HOME") ?? Path.Combine(home, provider == ProviderKind.Claude ? ".claude" : ".codex"), "CLI profile folder");
        var profile = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), ColumnSpacing = 5 }; profile.Children.Add(path);
        var browse = Ui.AsyncButton("Browse", async () => { if (await Folder(dialog) is { } folder) path.Text = folder; }); Grid.SetColumn(browse, 1); profile.Children.Add(browse);
        var pathLabel = Ui.Text("Profile", 12); Grid.SetRow(pathLabel, 1); fields.Children.Add(pathLabel); Grid.SetRow(profile, 1); Grid.SetColumn(profile, 1); fields.Children.Add(profile);
        content.Children.Add(fields);
        if (existing is null) content.Children.Add(Ui.Button("Create a separate profile", () => path.Text = Path.Combine(hub.Store.DirectoryPath, "profiles", provider.ToString().ToLowerInvariant(), Guid.NewGuid().ToString("N")), "Choose a new folder, then sign in to it after adding the account."));
        content.Children.Add(Ui.Text($"{provider} login and history stay in the selected profile folder.", 11, true));
        var error = Ui.Text("", 11); error.IsVisible = false; content.Children.Add(error);
        var buttons = Ui.Row(6); buttons.HorizontalAlignment = HorizontalAlignment.Right;
        buttons.Children.Add(Ui.Button("Cancel", () => dialog.Close()));
        buttons.Children.Add(Ui.Button(existing is null ? "Add account" : "Save", () =>
        {
            try
            {
                var account = ProfileRules.Create(hub.Settings.Accounts, provider, name.Text ?? "", path.Text ?? "", existing?.Id);
                if (!Directory.Exists(account.Directory)) Directory.CreateDirectory(account.Directory);
                dialog.Close(account);
            }
            catch (Exception ex) when (ex is ArgumentException or IOException or UnauthorizedAccessException or NotSupportedException)
            { error.Text = ex is ArgumentException ? ex.Message : "Could not open or create this folder."; error.IsVisible = true; }
        }));
        content.Children.Add(buttons); dialog.Content = content;
        return dialog;
    }

    public static Task<AccountProfile?> Account(Window owner, HubController hub, ProviderKind provider, AccountProfile? existing = null) => AccountWindow(hub, provider, existing).ShowDialog<AccountProfile?>(owner);

    public static async Task<ProjectFolder?> Project(Window owner, ProjectFolder? existing = null)
    {
        var dialog = Base(existing is null ? "Add project folder" : "Edit project folder");
        var content = Ui.Stack(10);
        var fields = new Grid { ColumnDefinitions = new ColumnDefinitions("66,*"), RowDefinitions = new RowDefinitions("Auto,Auto"), RowSpacing = 8, ColumnSpacing = 8 };
        var name = Input(existing?.Label, "Project name"); name.MaxLength = 80;
        var path = Input(existing?.Directory, "Project folder");
        fields.Children.Add(Ui.Text("Name", 12)); Grid.SetColumn(name, 1); fields.Children.Add(name);
        var folder = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), ColumnSpacing = 5 }; folder.Children.Add(path);
        var browse = Ui.AsyncButton("Browse", async () =>
        {
            if (await Folder(dialog) is not { } directory) return;
            path.Text = directory; if (string.IsNullOrWhiteSpace(name.Text)) name.Text = Path.GetFileName(directory);
        });
        Grid.SetColumn(browse, 1); folder.Children.Add(browse);
        var label = Ui.Text("Folder", 12); Grid.SetRow(label, 1); fields.Children.Add(label); Grid.SetRow(folder, 1); Grid.SetColumn(folder, 1); fields.Children.Add(folder); content.Children.Add(fields);
        var error = Ui.Text("", 11); error.IsVisible = false; content.Children.Add(error);
        var buttons = Ui.Row(6); buttons.HorizontalAlignment = HorizontalAlignment.Right;
        buttons.Children.Add(Ui.Button("Cancel", () => dialog.Close()));
        buttons.Children.Add(Ui.Button("Save", () =>
        {
            try
            {
                var directory = ProfileRules.NormalizeDirectory(path.Text ?? "");
                if (string.IsNullOrWhiteSpace(name.Text) || !Directory.Exists(directory)) { error.Text = "Enter a name and choose an existing folder."; error.IsVisible = true; return; }
                dialog.Close(new ProjectFolder(existing?.Id ?? Guid.NewGuid(), name.Text.Trim(), directory, existing?.IsHidden ?? false));
            }
            catch (Exception ex) when (ex is ArgumentException or IOException or NotSupportedException) { error.Text = "Choose a valid folder path."; error.IsVisible = true; }
        }));
        content.Children.Add(buttons); dialog.Content = content;
        return await dialog.ShowDialog<ProjectFolder?>(owner);
    }

    private static TextBox Input(string? text, string placeholder) => new() { Text = text, PlaceholderText = placeholder, FontSize = 12, MinHeight = 26, Padding = new Thickness(6, 3) };
    private static Window Base(string title) => new()
    {
        Title = title + " · AI SubHub", Width = 460, MaxHeight = 360, SizeToContent = SizeToContent.Height, CanResize = false,
        WindowStartupLocation = WindowStartupLocation.CenterOwner, Padding = new Thickness(14), Background = Ui.Background, Icon = App.Icon()
    };
}
