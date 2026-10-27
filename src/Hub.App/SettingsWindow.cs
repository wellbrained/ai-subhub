using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.Media;
using Hub.Core;

namespace Hub.App;

public sealed class SettingsWindow : Window
{
    private readonly HubController _hub;
    private readonly StackPanel _body = Ui.Stack(10);
    private readonly StackPanel _nav = Ui.Stack(3);
    private readonly TextBlock _error = Ui.Text("", 11);
    private readonly List<Border> _rows = [];
    private readonly ScrollViewer _scroll;
    private string _category = "Accounts";

    public SettingsWindow(HubController hub)
    {
        _hub = hub;
        Title = "Settings · AI SubHub"; Width = 600; Height = 320; MinWidth = 540; MinHeight = 260;
        Icon = App.Icon(); Background = Ui.Background; WindowStartupLocation = WindowStartupLocation.CenterOwner;
        var grid = new Grid { ColumnDefinitions = new ColumnDefinitions("100,*"), Margin = new Thickness(12), ColumnSpacing = 14 };
        var navBorder = new Border { Child = _nav, BorderBrush = Ui.Line, BorderThickness = new Thickness(0, 0, 1, 0), Padding = new Thickness(0, 0, 10, 0) };
        grid.Children.Add(navBorder);
        var content = new Grid { RowDefinitions = new RowDefinitions("*,Auto"), RowSpacing = 5 };
        _scroll = new ScrollViewer { Content = _body, HorizontalScrollBarVisibility = Avalonia.Controls.Primitives.ScrollBarVisibility.Disabled };
        content.Children.Add(_scroll); Grid.SetRow(_error, 1); content.Children.Add(_error); _error.IsVisible = false;
        Grid.SetColumn(content, 1); grid.Children.Add(content); Content = grid;
        Render();
    }

    public void SelectCategory(string category)
    {
        if (!new[] { "Accounts", "Projects", "General", "Tools", "About" }.Contains(category)) return;
        _category = category; _error.IsVisible = false; Render();
    }

    private void Render()
    {
        Background = Ui.Background; _body.Children.Clear(); _nav.Children.Clear(); _rows.Clear(); _body.Spacing = _category == "General" ? 6 : 10;
        _error.Foreground = Ui.Foreground;
        if (_nav.Parent is Border navigation) navigation.BorderBrush = Ui.Line;
        var title = Ui.Heading("Settings", 14); title.Margin = new Thickness(7, 1, 0, 10); _nav.Children.Add(title);
        foreach (var category in new[] { "Accounts", "Projects", "General", "Tools", "About" })
        {
            var item = Ui.Button(category, () => SelectCategory(category));
            item.HorizontalAlignment = HorizontalAlignment.Stretch; item.HorizontalContentAlignment = HorizontalAlignment.Left;
            item.MinHeight = 28; item.Padding = new Thickness(7, 4); item.BorderThickness = new Thickness(0); item.Background = category == _category ? Ui.Line : Brushes.Transparent;
            if (category == _category) item.Foreground = Ui.Accent;
            _nav.Children.Add(item);
        }
        switch (_category)
        {
            case "Accounts": Accounts(); break;
            case "Projects": Projects(); break;
            case "General": General(); break;
            case "Tools": Tools(); break;
            default: About(); break;
        }
    }

    private void Header(string title, Control? action = null)
    {
        var header = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), ColumnSpacing = 8 };
        header.Children.Add(Ui.Heading(title, 18));
        if (action is not null) { Grid.SetColumn(action, 1); header.Children.Add(action); }
        _body.Children.Add(header);
    }

    private void Accounts()
    {
        var add = Ui.Button("Add account", () => { });
        var menu = new MenuFlyout();
        foreach (var provider in Enum.GetValues<ProviderKind>())
        {
            var item = new MenuItem { Header = provider.ToString() };
            item.Click += async (_, _) =>
            {
                if (await Dialogs.Account(this, _hub, provider) is not { } account) return;
                _hub.Settings.Accounts.Add(account); Safe(_hub.Save); Render(); await _hub.RefreshAsync(true);
            };
            menu.Items.Add(item);
        }
        add.Flyout = menu; Header("Accounts", add);
        if (_hub.Settings.Accounts.Count == 0) _body.Children.Add(Ui.Text("Add a CLI profile to see its usage.", 12, true));
        foreach (var account in AccountCatalog.Ordered(_hub.Settings))
        {
            var actions = Ui.Row(5);
            foreach (var direction in new[] { -1, 1 })
            {
                var move = Ui.Button(direction < 0 ? "↑" : "↓", () => { AccountCatalog.Move(_hub.Settings, account.Id, direction); Safe(_hub.Save); Render(); });
                move.Padding = new Thickness(5, 2); move.IsEnabled = AccountCatalog.CanMove(_hub.Settings, account.Id, direction);
                ToolTip.SetTip(move, move.IsEnabled ? $"Move {(direction < 0 ? "up" : "down")} within {account.Provider}" : $"Already {(direction < 0 ? "first" : "last")} in {account.Provider}");
                actions.Children.Add(move);
            }
            actions.Children.Add(Ui.AsyncButton("Edit", async () =>
            {
                if (await Dialogs.Account(this, _hub, account.Provider, account) is not { } edited) return;
                var index = _hub.Settings.Accounts.FindIndex(a => a.Id == account.Id);
                if (index < 0) return;
                if (!StringComparer.OrdinalIgnoreCase.Equals(edited.Directory, account.Directory)) _hub.ForgetReading(account.Id);
                _hub.Settings.Accounts[index] = edited; Safe(_hub.Save); Render(); await _hub.RefreshAsync(true);
            }));
            actions.Children.Add(More(
                ("Monthly billing…", () => _ = MonthlyBillingUi.Edit(this, _hub, account), "Set a monthly renewal or end date and reminders."),
                ("Open billing", () => _ = MonthlyBillingUi.OpenBilling(this, _hub, account), "Opens the provider in your browser; check its signed-in account."),
                ("Sign in", () => Safe(() => _hub.SignIn(account)), "Sign in to this CLI profile."),
                ("Remove account", () => { Safe(() => _hub.RemoveAccount(account.Id)); Render(); }, "Remove from AI SubHub. Keep the CLI login, folder and history.")));
            var grid = new Grid { ColumnDefinitions = new ColumnDefinitions("*,55,Auto"), ColumnSpacing = 8 };
            grid.Children.Add(Ui.AccountName(account.Label, _hub.Plans.GetValueOrDefault(account.Id, "Plan unknown"), 13, 130, Ui.AccountHint(account.Label, _hub.Results.GetValueOrDefault(account.Id)) + "\n" + account.Directory));
            var provider = Ui.Text(account.Provider.ToString(), 11, true); Grid.SetColumn(provider, 1); grid.Children.Add(provider);
            Grid.SetColumn(actions, 2); grid.Children.Add(actions); Row(grid);
        }
        _body.Children.Add(Ui.Text("↑/↓ orders accounts within each provider. Edit to rename.", 11, true));
    }

    private void Projects()
    {
        var add = Ui.AsyncButton("Add folder", async () =>
        {
            if (await Dialogs.Project(this) is not { } project) return;
            if (_hub.Settings.Projects.Any(p => StringComparer.OrdinalIgnoreCase.Equals(p.Directory, project.Directory))) { Error("That folder is already saved."); return; }
            _hub.Settings.Projects.Add(project); _hub.Settings.SelectedProjectId = project.Id; Safe(_hub.Save); Render();
        });
        Header("Projects", add);
        _body.Children.Add(Ui.Text("Your order is used in Launch. Hidden folders stay saved here.", 11, true));
        foreach (var project in _hub.Settings.Projects.ToArray())
        {
            var identity = Ui.Stack(2); identity.Children.Add(Clipped(project.Label, project.Directory, 13)); identity.Children.Add(Clipped(project.Directory, project.Directory, 11, true));
            var actions = Ui.Row(5);
            var up = Ui.Button("↑", () => { ProjectCatalog.Move(_hub.Settings, project.Id, -1); Safe(_hub.Save); Render(); }, "Move up in the project and Launch lists.");
            up.Padding = new Thickness(5, 2); up.IsEnabled = _hub.Settings.Projects[0].Id != project.Id;
            ToolTip.SetTip(up, up.IsEnabled ? "Move up" : "Already first"); actions.Children.Add(up);
            var down = Ui.Button("↓", () => { ProjectCatalog.Move(_hub.Settings, project.Id, 1); Safe(_hub.Save); Render(); }, "Move down in the project and Launch lists.");
            down.Padding = new Thickness(5, 2); down.IsEnabled = _hub.Settings.Projects[^1].Id != project.Id;
            ToolTip.SetTip(down, down.IsEnabled ? "Move down" : "Already last"); actions.Children.Add(down);
            var shown = new CheckBox { IsChecked = !project.IsHidden, Content = "Show", FontSize = 11, MinHeight = 26 };
            ToolTip.SetTip(shown, "Show in Launch menus. Uncheck to hide without removing this project.");
            shown.IsCheckedChanged += (_, _) => { ProjectCatalog.SetHidden(_hub.Settings, project.Id, shown.IsChecked != true); Safe(_hub.Save); Render(); };
            actions.Children.Add(shown);
            actions.Children.Add(Ui.AsyncButton("Edit", async () =>
            {
                if (await Dialogs.Project(this, project) is not { } edited) return;
                if (_hub.Settings.Projects.Any(p => p.Id != project.Id && StringComparer.OrdinalIgnoreCase.Equals(p.Directory, edited.Directory))) { Error("That folder is already saved."); return; }
                var index = _hub.Settings.Projects.FindIndex(p => p.Id == project.Id);
                if (index >= 0) _hub.Settings.Projects[index] = edited;
                Safe(_hub.Save); Render();
            }));
            actions.Children.Add(More(("Remove folder", () =>
            {
                _hub.Settings.Projects.RemoveAll(p => p.Id == project.Id);
                if (_hub.Settings.SelectedProjectId == project.Id) _hub.Settings.SelectedProjectId = null;
                Safe(_hub.Save); Render();
            }, "Remove the shortcut. Keep its files.")));
            var grid = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), ColumnSpacing = 8 }; grid.Children.Add(identity); Grid.SetColumn(actions, 1); grid.Children.Add(actions); Row(grid);
        }
        if (_hub.Settings.Projects.Count == 0) _body.Children.Add(Ui.Text("No saved folders yet.", 12, true));
    }

    private void General()
    {
        Header("General");
        var themes = Choice(["Dark", "Light", "System"], _hub.Settings.Theme);
        themes.SelectionChanged += (_, _) => { if (themes.SelectedItem is string theme) { Safe(() => _hub.UpdateTheme(theme)); Render(); } };
        Field("Appearance", themes);
        var usageStyle = Choice(["% remaining", "% used"], _hub.Settings.UsageStyle == "Used" ? "% used" : "% remaining");
        usageStyle.SelectionChanged += (_, _) => { if (usageStyle.SelectedItem is string selected) { _hub.Settings.UsageStyle = selected == "% used" ? "Used" : "Remaining"; Safe(_hub.Save); } };
        ToolTip.SetTip(usageStyle, "Choose what percentages and bar fill represent. Applies immediately to the window and tray tooltip."); Field("Usage display", usageStyle);
        var interval = new NumericUpDown { Minimum = 1, Maximum = 60, Value = _hub.Settings.RefreshMinutes, Increment = 1, FormatString = "0", MinHeight = 26, FontSize = 12, Padding = new Thickness(6, 3) };
        interval.ValueChanged += (_, _) => { if (interval.Value is { } minutes) { _hub.Settings.RefreshMinutes = (int)minutes; Safe(_hub.Save); } };
        ToolTip.SetTip(interval, "Minutes between refreshes. Errors and provider rate limits increase the delay."); Field("Refresh (minutes)", interval);
        var terminal = Choice(OperatingSystem.IsMacOS() ? ["Terminal.app"] : ["Windows Terminal", "PowerShell"], OperatingSystem.IsMacOS() ? "Terminal.app" : _hub.Settings.Terminal);
        terminal.SelectionChanged += (_, _) => { if (terminal.SelectedItem is string selected) { _hub.Settings.Terminal = selected; Safe(_hub.Save); } };
        ToolTip.SetTip(terminal, "Default terminal for sign-in. The Launch menu lets you choose per session."); Field("Sign-in terminal", terminal);
        var startup = new CheckBox { Content = "Enabled", IsChecked = _hub.Settings.StartWithWindows, IsEnabled = !_hub.IsDemo, FontSize = 12, MinHeight = 26 };
        startup.IsCheckedChanged += (_, _) =>
        {
            if (startup.IsChecked == _hub.Settings.StartWithWindows) return;
            Safe(() => _hub.SetStartup(startup.IsChecked == true));
            startup.IsChecked = _hub.Settings.StartWithWindows;
        };
        ToolTip.SetTip(startup, _hub.IsDemo ? "Startup changes are disabled in demo mode." : "Open in the tray or menu bar when you sign in. No administrator rights needed."); Field(OperatingSystem.IsMacOS() ? "Start at login" : "Start with Windows", startup);
        _body.Children.Add(Ui.Text("Close the main window to keep running. Exit from the tray menu.", 11, true));
    }

    private void Tools()
    {
        Header("Tools");
        foreach (var provider in Enum.GetValues<ProviderKind>())
        {
            var section = Ui.Stack(5); section.Children.Add(Ui.Text($"{provider} CLI", 12));
            var path = new TextBox { Text = provider == ProviderKind.Claude ? _hub.Settings.ClaudeExecutable : _hub.Settings.CodexExecutable, PlaceholderText = "Auto-detect executable", FontSize = 12, MinHeight = 26, Padding = new Thickness(6, 3) };
            var line = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), ColumnSpacing = 6 }; line.Children.Add(path);
            var save = Ui.Button("Save", () =>
            {
                var value = string.IsNullOrWhiteSpace(path.Text) ? null : path.Text.Trim().Trim('"');
                if (value is not null && (!File.Exists(value) || OperatingSystem.IsWindows() && !new[] { ".exe", ".cmd", ".bat" }.Contains(Path.GetExtension(value), StringComparer.OrdinalIgnoreCase))) { Error("Choose an existing CLI executable, or leave empty."); return; }
                if (provider == ProviderKind.Claude) _hub.Settings.ClaudeExecutable = value; else _hub.Settings.CodexExecutable = value;
                Safe(_hub.Save); Render();
            });
            Grid.SetColumn(save, 1); line.Children.Add(save); section.Children.Add(line);
            var detected = _hub.Launcher.Executable(provider);
            var status = Ui.Text(detected is null ? "Not found — install the CLI or enter its path." : "Executable found", 11, true); ToolTip.SetTip(status, detected); section.Children.Add(status); _body.Children.Add(section);
        }
        _body.Children.Add(Ui.Text("Leave empty to detect installed tools automatically.", 11, true));
    }

    private void About()
    {
        Header("AI SubHub");
        _body.Children.Add(Ui.Text("Version 0.13.2 · Free and open source · MIT", 12, true));
        _body.Children.Add(Ui.Text("Usage goes directly to your providers. No SubHub account, backend or telemetry.", 12));
        _body.Children.Add(Ui.Text("Claude usage uses an unofficial endpoint; provider changes can interrupt it. Logins stay with the CLIs.", 11, true));
        _body.Children.Add(Ui.Text("Settings location", 12)); _body.Children.Add(Clipped(_hub.Store.DirectoryPath, _hub.Store.DirectoryPath, 11, true));
    }

    private void Row(Control content, bool compact = false)
    {
        var row = new Border { Child = content, BorderBrush = Ui.Line, BorderThickness = new Thickness(0, 0, 0, 1), Padding = compact ? new Thickness(0, 3, 0, 4) : new Thickness(0, 5, 0, 7) };
        _rows.Add(row); _body.Children.Add(row);
    }

    private void Field(string label, Control editor)
    {
        var row = new Grid { ColumnDefinitions = new ColumnDefinitions("*,170"), ColumnSpacing = 10, MinHeight = 30 };
        row.Children.Add(Ui.Text(label, 12)); Grid.SetColumn(editor, 1); row.Children.Add(editor); Row(row, true);
    }

    private static ComboBox Choice(string[] options, string value) => new() { ItemsSource = options, SelectedItem = value, HorizontalAlignment = HorizontalAlignment.Stretch, MinHeight = 26, FontSize = 12, Padding = new Thickness(6, 3) };
    private static TextBlock Clipped(string text, string hint, double size, bool muted = false)
    {
        var label = Ui.Text(text, size, muted); label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis; ToolTip.SetTip(label, hint); return label;
    }

    private static Button More(params (string Label, Action Action, string Hint)[] actions)
    {
        var button = Ui.Button("⋯", () => { }, "More actions"); button.Padding = new Thickness(7, 2);
        var menu = new MenuFlyout();
        foreach (var action in actions)
        {
            var item = new MenuItem { Header = action.Label }; ToolTip.SetTip(item, action.Hint); item.Click += (_, _) => action.Action(); menu.Items.Add(item);
        }
        button.Flyout = menu; return button;
    }

    private void Error(string message) { _error.Text = message; _error.IsVisible = true; }
    private void Safe(Action action)
    {
        try { action(); _error.IsVisible = false; }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException or System.ComponentModel.Win32Exception or System.Security.SecurityException)
        { Error(ex is InvalidOperationException ? ex.Message : "Could not save. Check folder permissions."); }
    }

    public object LayoutReport()
    {
        var viewportBottom = _scroll.TranslatePoint(new Point(0, _scroll.Bounds.Height), this)?.Y ?? 0;
        return new { Category = _category, WindowWidth = Bounds.Width, WindowHeight = Bounds.Height, RowCount = _rows.Count, TallestRow = _rows.Count == 0 ? 0 : _rows.Max(r => r.Bounds.Height), AllRowsFit = _rows.All(r => r.TranslatePoint(new Point(0, r.Bounds.Height), this)?.Y <= viewportBottom), ContentFits = _scroll.Extent.Height <= _scroll.Viewport.Height + 1 };
    }
}
