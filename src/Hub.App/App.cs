using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Markup.Xaml.Styling;
using Avalonia.Platform;
using Avalonia.Media.Imaging;
using Avalonia.Styling;
using Avalonia.Themes.Fluent;
using Avalonia.Threading;

namespace Hub.App;

public sealed class App : Application
{
    private HubController? _controller;
    private TrayIcon? _tray;
    private MainWindow? _window;
    public override void Initialize()
    {
        Styles.Add(new FluentTheme());
    }

    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
        {
            desktop.ShutdownMode = ShutdownMode.OnExplicitShutdown;
            var isolated = desktop.Args?.Contains("--demo") == true;
            _controller = new HubController(isolated);
            SetTheme(desktop.Args?.Contains("--light") == true && isolated ? "Light" : _controller.Settings.Theme);
            _window = new MainWindow(_controller);
            desktop.MainWindow = _window;
            Program.OnActivation(() => Dispatcher.UIThread.Post(ShowWindow));
            _window.Closing += (_, e) => { e.Cancel = true; _window.Hide(); };
            var menu = new NativeMenu();
            var show = new NativeMenuItem("Open AI SubHub");
            show.Click += (_, _) => ShowWindow();
            var refresh = new NativeMenuItem("Refresh usage");
            refresh.Click += async (_, _) => await _controller.RefreshAsync(true);
            var exit = new NativeMenuItem("Exit");
            exit.Click += (_, _) => desktop.Shutdown();
            menu.Items.Add(show); menu.Items.Add(refresh); menu.Items.Add(new NativeMenuItemSeparator()); menu.Items.Add(exit);
            _tray = new TrayIcon { Icon = Icon(), ToolTipText = "AI SubHub", Menu = menu, IsVisible = true };
            void UpdateTray() => _tray.ToolTipText = Hub.Core.TraySummary.Build(_controller.Settings, _controller.Results, DateTimeOffset.UtcNow);
            _controller.Changed += UpdateTray; UpdateTray();
            _tray.Clicked += (_, _) => ShowWindow();
            TrayIcon.SetIcons(this, new TrayIcons { _tray });
            desktop.Exit += (_, _) => { _controller.Changed -= UpdateTray; _controller.Dispose(); _tray.Dispose(); };
            // Avalonia normally shows MainWindow automatically; hide only after startup is complete.
            if (desktop.Args?.Contains("--tray") == true) Dispatcher.UIThread.Post(() => { if (!Program.ActivationRequested) _window.Hide(); }, DispatcherPriority.Background);
            _controller.Start();
            if (desktop.Args?.Contains("--ui-smoke") == true)
            {
                Window snapshotTarget = _window;
                SettingsWindow? settingsTarget = null;
                if (desktop.Args.Contains("--settings"))
                {
                    var settings = new SettingsWindow(_controller);
                    var categoryIndex = Array.IndexOf(desktop.Args, "--category");
                    if (categoryIndex >= 0 && categoryIndex + 1 < desktop.Args.Length) settings.SelectCategory(desktop.Args[categoryIndex + 1]);
                    Dispatcher.UIThread.Post(() => settings.Show(_window), DispatcherPriority.Background);
                    snapshotTarget = settings; settingsTarget = settings;
                }
                if (desktop.Args.Contains("--account-dialog"))
                {
                    var dialog = Dialogs.AccountWindow(_controller, Hub.Core.ProviderKind.Claude, _controller.Settings.Accounts.FirstOrDefault());
                    Dispatcher.UIThread.Post(() => dialog.Show(_window), DispatcherPriority.Background); snapshotTarget = dialog;
                }
                DispatcherTimer.RunOnce(() =>
                {
                    var lifecycleIndex = Array.IndexOf(desktop.Args, "--lifecycle-report");
                    if (lifecycleIndex >= 0 && lifecycleIndex + 1 < desktop.Args.Length)
                    {
                        var visibleBeforeClose = _window.IsVisible;
                        _window.Close();
                        var hiddenOnClose = !_window.IsVisible;
                        ShowWindow();
                        File.WriteAllText(desktop.Args[lifecycleIndex + 1], System.Text.Json.JsonSerializer.Serialize(new { ActivationRequested = Program.ActivationRequested, VisibleBeforeClose = visibleBeforeClose, HiddenOnClose = hiddenOnClose, Reopened = _window.IsVisible, TrayConfigured = _tray.IsVisible }));
                    }
                    var index = Array.IndexOf(desktop.Args, "--snapshot");
                    var trayIndex = Array.IndexOf(desktop.Args, "--tray-report");
                    if (isolated && trayIndex >= 0 && trayIndex + 1 < desktop.Args.Length)
                        File.WriteAllText(desktop.Args[trayIndex + 1], System.Text.Json.JsonSerializer.Serialize(new { Text = _tray.ToolTipText, Length = _tray.ToolTipText?.Length }));
                    var layoutIndex = Array.IndexOf(desktop.Args, "--layout-report");
                    if (layoutIndex >= 0 && layoutIndex + 1 < desktop.Args.Length)
                        File.WriteAllText(desktop.Args[layoutIndex + 1], System.Text.Json.JsonSerializer.Serialize(settingsTarget is not null ? settingsTarget.LayoutReport() : _window.LayoutReport()));
                    if (index >= 0 && index + 1 < desktop.Args.Length)
                    {
                        using var bitmap = new RenderTargetBitmap(new PixelSize((int)snapshotTarget.Bounds.Width, (int)snapshotTarget.Bounds.Height), new Vector(96, 96));
                        bitmap.Render(snapshotTarget); bitmap.Save(desktop.Args[index + 1], PngBitmapEncoderOptions.Default);
                    }
                    desktop.Shutdown();
                }, TimeSpan.FromSeconds(4));
            }
        }
        base.OnFrameworkInitializationCompleted();
    }

    public void ShowWindow()
    {
        if (_window is null) return;
        _window.Show(); _window.WindowState = WindowState.Normal; _window.Activate();
    }

    public void SetTheme(string theme) => RequestedThemeVariant = theme switch { "Light" => ThemeVariant.Light, "System" => ThemeVariant.Default, _ => ThemeVariant.Dark };
    public static WindowIcon Icon() => new(AssetLoader.Open(new Uri("avares://AI.SubHub/Assets/hub.ico")));
}
