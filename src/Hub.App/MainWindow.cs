using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.Media;
using Hub.Core;

namespace Hub.App;

public sealed class MainWindow : Window
{
    private readonly HubController _hub;
    private readonly StackPanel _accounts = Ui.Stack(10);
    private readonly TextBlock _notice = Ui.Text("", 11, true);
    private readonly Button _refresh;
    private readonly ScrollViewer _scroll;
    private readonly List<Border> _accountRows = [];
    private readonly List<MenuFlyout> _launchMenus = [];
    private SettingsWindow? _settings;

    public MainWindow(HubController hub)
    {
        _hub = hub;
        Title = "AI SubHub"; Width = 620; Height = 404; MinWidth = 620; MinHeight = 300;
        Icon = App.Icon(); FontFamily = "Segoe UI";
        _refresh = Ui.AsyncButton("Refresh", () => _hub.RefreshAsync(true)); Compact(_refresh);
        var root = new Grid { RowDefinitions = new RowDefinitions("Auto,*,Auto"), Margin = new Thickness(12), RowSpacing = 10 };
        var top = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), ColumnSpacing = 12 };
        top.Children.Add(Ui.Heading("AI SubHub", 20));
        var actions = Ui.Row(6); actions.Children.Add(_refresh);
        var settings = Ui.Button("Settings", OpenSettings); Compact(settings); actions.Children.Add(settings);
        Grid.SetColumn(actions, 1); top.Children.Add(actions); root.Children.Add(top);

        _scroll = new ScrollViewer { Content = _accounts, HorizontalScrollBarVisibility = Avalonia.Controls.Primitives.ScrollBarVisibility.Disabled };
        Grid.SetRow(_scroll, 1); root.Children.Add(_scroll);
        Grid.SetRow(_notice, 2); root.Children.Add(_notice);
        Content = root; _hub.Changed += Render; Render();
    }

    private static void Compact(Button button)
    {
        button.FontSize = 12; button.Padding = new Thickness(8, 3); button.MinHeight = 24;
    }

    private void OpenSettings() => OpenSettings("Accounts");

    private void OpenSettings(string category)
    {
        if (_settings is { IsVisible: true }) { _settings.SelectCategory(category); _settings.Activate(); return; }
        _settings = new SettingsWindow(_hub); _settings.SelectCategory(category); _settings.Show(this);
    }

    private async Task AddAccount(ProviderKind provider)
    {
        if (await Dialogs.Account(this, _hub, provider) is not { } account) return;
        _hub.Settings.Accounts.Add(account); Safe(_hub.Save); await _hub.RefreshAsync(true);
    }

    private void Render()
    {
        Background = Ui.Background;
        _notice.Foreground = Ui.Muted;
        _refresh.Content = _hub.IsRefreshing ? "Refreshing…" : "Refresh";
        _refresh.IsEnabled = !_hub.IsRefreshing && !_hub.IsDemo;
        ToolTip.SetTip(_refresh, _hub.IsDemo ? "Demo readings are simulated." : _hub.IsRefreshing ? "A refresh is already running." : "Request fresh usage readings.");
        _notice.Text = _hub.Notice ?? (_hub.IsDemo ? "Demo · Simulated readings" : $"v0.7.0 · Refresh every {_hub.Settings.RefreshMinutes}m · Close to tray");
        _accounts.Children.Clear(); _accountRows.Clear(); _launchMenus.Clear();
        foreach (var provider in Enum.GetValues<ProviderKind>())
        {
            var section = Ui.Stack(6);
            var profiles = _hub.Settings.Accounts.Where(a => a.Provider == provider).ToList();
            var heading = Ui.Row(8);
            heading.Children.Add(Ui.Heading(provider == ProviderKind.Claude ? "Claude" : "Codex", 14));
            var add = Ui.AsyncButton("Add account", () => AddAccount(provider));
            Compact(add); add.FontSize = 11; add.Padding = new Thickness(6, 2); add.MinHeight = 22; add.Background = Brushes.Transparent;
            heading.Children.Add(add); section.Children.Add(heading);
            if (profiles.Count == 0) section.Children.Add(Ui.Text("No accounts connected.", 12, true));
            foreach (var account in profiles)
            {
                var row = AccountRow(account); _accountRows.Add(row); section.Children.Add(row);
            }
            _accounts.Children.Add(section);
        }
    }

    private Border AccountRow(AccountProfile account)
    {
        var result = _hub.Results.GetValueOrDefault(account.Id);
        var snapshot = result?.Snapshot;
        var stale = result?.State != UsageState.Ready || snapshot is not null && DateTimeOffset.UtcNow - snapshot.FetchedAt > TimeSpan.FromMinutes(_hub.Settings.RefreshMinutes + 1);
        var body = Ui.Stack(4);
        var header = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto,Auto"), ColumnSpacing = 10 };
        header.Children.Add(Ui.AccountName(account.Label, _hub.Plans.GetValueOrDefault(account.Id, "Plan unknown"), 14, 220, account.Label));
        var age = Ui.Text(snapshot is null ? "" : $"{(stale ? "Stale · " : "")}{Ui.Age(snapshot.FetchedAt)}", 11, true);
        if (snapshot is not null) ToolTip.SetTip(age, $"Last successful refresh: {snapshot.FetchedAt.LocalDateTime:g}{(stale ? ". Last known reading, not current usage." : "")}");
        Grid.SetColumn(age, 1); header.Children.Add(age);
        var launch = LaunchButton(account); Grid.SetColumn(launch, 2); header.Children.Add(launch); body.Children.Add(header);
        if (result?.State == UsageState.SignInRequired)
        {
            var signIn = Ui.Button("Sign in again", () => Safe(() => _hub.Launcher.Launch(account, Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), true))); Compact(signIn); body.Children.Add(signIn);
        }
        if (result is { State: not UsageState.Ready }) body.Children.Add(Ui.Text(result.Message, 11, true));
        if (snapshot is not null)
        {
            foreach (var window in snapshot.Windows)
            {
                var quota = new Grid { ColumnDefinitions = new ColumnDefinitions("110,*,82,125"), ColumnSpacing = 8 };
                var label = Ui.Text(QuotaParser.CompactLabel(window.Label), 11, true); label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis;
                ToolTip.SetTip(label, window.Label); quota.Children.Add(label);
                var progress = new ProgressBar { Minimum = 0, Maximum = 100, Value = window.RemainingPercent, Height = 4, VerticalAlignment = VerticalAlignment.Center, Foreground = stale ? Ui.Muted : window.RemainingPercent < 20 ? Ui.Brush("#D89B56") : Ui.Accent };
                Grid.SetColumn(progress, 1); quota.Children.Add(progress);
                var percent = Ui.Text($"{window.RemainingPercent:0.#}% left", 12); percent.HorizontalAlignment = HorizontalAlignment.Right;
                Grid.SetColumn(percent, 2); quota.Children.Add(percent);
                var fullReset = Ui.ResetText(window.ResetsAt);
                var reset = Ui.Text(fullReset.Split(" · ")[0], 11, true); reset.TextWrapping = TextWrapping.NoWrap; reset.TextTrimming = TextTrimming.CharacterEllipsis;
                ToolTip.SetTip(reset, fullReset); Grid.SetColumn(reset, 3); quota.Children.Add(reset); body.Children.Add(quota);
            }
        }
        else if (result is null || result.State == UsageState.Ready) body.Children.Add(Ui.Text(_hub.IsRefreshing ? "Connecting…" : "Usage unavailable", 11, true));
        var card = Ui.Card(body); card.Padding = new Thickness(10, 8); card.CornerRadius = new CornerRadius(7); return card;
    }

    private Button LaunchButton(AccountProfile account)
    {
        var button = Ui.Button("Launch ▾", () => { }); Compact(button);
        var menu = new MenuFlyout();
        _launchMenus.Add(menu);
        foreach (var project in ProjectCatalog.Visible(_hub.Settings))
        {
            var item = new MenuItem { Header = project.Label }; ToolTip.SetTip(item, project.Directory);
            foreach (var target in ProjectTargets(account, project)) item.Items.Add(target);
            menu.Items.Add(item);
        }
        if (menu.Items.Count == 0) menu.Items.Add(new MenuItem { Header = "No visible projects", IsEnabled = false });
        menu.Items.Add(new Separator());
        var manage = new MenuItem { Header = "Manage projects…" }; manage.Click += (_, _) => OpenSettings("Projects"); menu.Items.Add(manage);
        button.Flyout = menu;
        return button;
    }

    private IReadOnlyList<Control> ProjectTargets(AccountProfile account, ProjectFolder project)
    {
        var targets = new List<Control>();
        var cli = _hub.Launcher.Executable(account.Provider) is not null;
        var folderExists = _hub.IsDemo || Directory.Exists(project.Directory);
        Add("Windows Terminal", () => _hub.Launcher.Launch(account, project.Directory, target: TerminalTarget.WindowsTerminal), cli && Executables.Find("wt") is not null, "Run this account's CLI in this folder using Windows Terminal.");
        Add("PowerShell", () => _hub.Launcher.Launch(account, project.Directory, target: TerminalTarget.PowerShell), cli, "Run this account's CLI in this folder using PowerShell.");
        targets.Add(new Separator());
        Add("VS Code", () => _hub.Launcher.OpenCode(project.Directory), Executables.Find("code") is not null, "Open this folder in VS Code. Its extension login is separate from the CLI profile.");
        var desktop = _hub.DesktopApplications.GetValueOrDefault(account.Provider);
        if (account.Provider == ProviderKind.Claude)
            Add("Claude Desktop · local folder", () => _hub.Launcher.OpenClaudeDesktopFolder(project.Directory), desktop is not null, "Open this local folder in Claude Code inside Desktop, using Desktop's login. Claude asks you to confirm the folder. This is not a Claude chat Project.");
        else
            Add("Codex Desktop · choose folder there", () => { desktop!.Launch(); _hub.Notice = $"Codex Desktop opened. Choose {project.Label} there."; Render(); }, desktop is not null, "Open Codex Desktop with its own login. Windows folder navigation is not available through a verified interface; select this folder inside the app.");
        return targets;

        void Add(string label, Action action, bool enabled, string hint)
        {
            var item = new MenuItem { Header = label, IsEnabled = enabled && folderExists && !_hub.IsDemo };
            ToolTip.SetTip(item, _hub.IsDemo ? "Demo mode does not launch external apps." : !folderExists ? "This folder no longer exists. Edit it in Settings → Projects." : !enabled ? "Install this application, then restart AI SubHub." : hint);
            item.Click += (_, _) => Safe(action); targets.Add(item);
        }
    }

    public object LayoutReport()
    {
        var viewportBottom = _scroll.TranslatePoint(new Point(0, _scroll.Bounds.Height), this)?.Y ?? 0;
        var projects = _launchMenus.FirstOrDefault()?.Items.OfType<MenuItem>().Where(i => i.Items.Count > 0).ToArray() ?? [];
        return new { WindowHeight = Bounds.Height, AccountCount = _accountRows.Count, TallestAccountRow = _accountRows.Count == 0 ? 0 : _accountRows.Max(r => r.Bounds.Height), AllAccountsFit = _accountRows.All(r => r.TranslatePoint(new Point(0, r.Bounds.Height), this)?.Y <= viewportBottom), VisibleProjects = projects.Select(i => i.Header?.ToString()).ToArray(), HiddenProjects = _hub.Settings.Projects.Count(p => p.IsHidden), LaunchTargets = projects.FirstOrDefault()?.Items.OfType<MenuItem>().Select(i => i.Header?.ToString()).ToArray() };
    }

    private void Safe(Action action)
    {
        try { action(); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException or System.ComponentModel.Win32Exception)
        { _hub.Notice = ex is InvalidOperationException ? ex.Message : "The action could not be completed. Check the selected folders and tool settings."; Render(); }
    }
}
