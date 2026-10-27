using Avalonia;
using Avalonia.Controls;
using Avalonia.Threading;
using Hub.Core;

namespace Hub.App;

internal sealed class WindowPlacementTracker
{
    private readonly Window _window;
    private readonly HubController _hub;
    private readonly DispatcherTimer _saveTimer = new() { Interval = TimeSpan.FromMilliseconds(400) };
    private WindowPlacement? _normal;
    private WindowPlacement? _saved;
    private bool _maximized;
    private bool _opened;

    public WindowPlacementTracker(Window window, HubController hub)
    {
        _window = window; _hub = hub;
        _saved = hub.Settings.MainWindowPlacement;
        if (hub.Settings.MainWindowPlacement is { } saved && Valid(saved))
        {
            var screen = window.Screens.ScreenFromPoint(new PixelPoint(saved.X, saved.Y)) ?? window.Screens.Primary;
            _normal = screen is null ? saved : Fit(saved, screen.WorkingArea, screen.Scaling);
            window.Width = _normal.Width; window.Height = _normal.Height;
            window.Position = new PixelPoint(_normal.X, _normal.Y);
            window.WindowStartupLocation = WindowStartupLocation.Manual;
            _maximized = saved.Maximized;
            if (_maximized) window.WindowState = WindowState.Maximized;
        }
        window.Opened += (_, _) => { _opened = true; QueueCapture(); };
        window.PositionChanged += (_, _) => QueueCapture();
        window.SizeChanged += (_, _) => QueueCapture();
        window.PropertyChanged += (_, e) => { if (e.Property == Window.WindowStateProperty) QueueCapture(); };
        window.Closing += (_, _) => Save();
        window.Closed += (_, _) => { _opened = false; _saveTimer.Stop(); };
        _saveTimer.Tick += (_, _) => Save();
    }

    // Defer until native resize/state notifications settle, so maximize/minimize bounds
    // cannot replace the last normal bounds. Hidden tray windows never supply geometry.
    private void QueueCapture()
    {
        if (!_opened) return;
        Dispatcher.UIThread.Post(() =>
        {
            if (!_opened) return;
            Capture();
            _saveTimer.Stop(); _saveTimer.Start();
        }, DispatcherPriority.Background);
    }

    private void Capture()
    {
        if (!_opened || !_window.IsVisible) return;
        if (_window.WindowState == WindowState.Maximized) _maximized = true;
        if (_window.WindowState != WindowState.Normal) return;
        _maximized = false;
        var size = _window.ClientSize;
        var placement = new WindowPlacement(_window.Position.X, _window.Position.Y, size.Width, size.Height);
        if (Valid(placement)) _normal = placement;
    }

    public void Save()
    {
        _saveTimer.Stop(); Capture();
        if (_normal is null || _hub.IsDemo) return;
        var placement = _normal with { Maximized = _maximized };
        if (_saved == placement) return;
        _hub.Settings.MainWindowPlacement = placement;
        try { _hub.Store.Save(_hub.Settings); _saved = placement; }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        { _hub.Notice = "Window placement could not be saved. Check access to the settings folder."; }
    }

    internal static bool Valid(WindowPlacement p) => double.IsFinite(p.Width) && double.IsFinite(p.Height)
        && p.Width >= 620 && p.Height >= 300;

    internal static WindowPlacement Fit(WindowPlacement p, PixelRect workArea, double scaling)
    {
        // Leave room for native borders/title bars. Positions are physical pixels;
        // Avalonia client sizes are logical pixels, including on mixed-DPI monitors.
        var width = Math.Clamp(p.Width, 620, Math.Max(620, workArea.Width / scaling - 16));
        var height = Math.Clamp(p.Height, 300, Math.Max(300, workArea.Height / scaling - 48));
        var x = (int)Math.Clamp((long)p.X, workArea.X, Math.Max((long)workArea.X, (long)workArea.Right - (long)Math.Ceiling((width + 16) * scaling)));
        var y = (int)Math.Clamp((long)p.Y, workArea.Y, Math.Max((long)workArea.Y, (long)workArea.Bottom - (long)Math.Ceiling((height + 48) * scaling)));
        return p with { X = x, Y = y, Width = width, Height = height };
    }

    internal void Show()
    {
        if (_window.WindowState == WindowState.Minimized)
            _window.WindowState = _maximized ? WindowState.Maximized : WindowState.Normal;
        _window.Show();
        _window.Activate();
    }
}
