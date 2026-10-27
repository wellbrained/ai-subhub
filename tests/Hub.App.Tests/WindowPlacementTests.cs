using Avalonia;
using Avalonia.Controls;
using Avalonia.Threading;
using Hub.Core;
using Xunit;

namespace Hub.App.Tests;

public sealed class WindowPlacementTests
{
    [Fact]
    public async Task MainWindowRestoresItsSizeAndPositionAfterRestart()
    {
        if (!OperatingSystem.IsWindows()) return;
        // Avalonia owns one UI dispatcher per process. Controller-only tests also
        // create timers, so exercise native windows in a separate test process.
        const string isolated = "AISUBHUB_NATIVE_PLACEMENT_TEST";
        if (Environment.GetEnvironmentVariable(isolated) != "1")
        {
            var bundled = Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "../../../../../.tools/dotnet/dotnet.exe"));
            var start = new System.Diagnostics.ProcessStartInfo(File.Exists(bundled) ? bundled : "dotnet")
            { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true };
            start.ArgumentList.Add("vstest");
            start.ArgumentList.Add(typeof(WindowPlacementTests).Assembly.Location);
            start.ArgumentList.Add("--TestCaseFilter:FullyQualifiedName=Hub.App.Tests.WindowPlacementTests.MainWindowRestoresItsSizeAndPositionAfterRestart");
            start.Environment[isolated] = "1";
            using var process = System.Diagnostics.Process.Start(start)!;
            var output = process.StandardOutput.ReadToEndAsync();
            var error = process.StandardError.ReadToEndAsync();
            try { await process.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(40)); }
            catch { process.Kill(entireProcessTree: true); throw; }
            Assert.True(process.ExitCode == 0, await output + await error);
            return;
        }
        var completion = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread = new Thread(() =>
        {
            var directory = Path.Combine(Path.GetTempPath(), "hub-placement-" + Guid.NewGuid());
            try
            {
                AppBuilder.Configure<App>().UsePlatformDetect().SetupWithoutStarting();
                var store = new SettingsStore(directory);
                store.Save(new());
                using (var hub = new HubController(false, store, null, null))
                {
                    var window = new MainWindow(hub);
                    window.Show(); Pump();
                    window.Width = 760; window.Height = 470;
                    window.Position = new PixelPoint(120, 140); Pump();
                    window.Close(); Pump();
                }
                using (var hub = new HubController(false, store, null, null))
                {
                    var window = new MainWindow(hub);
                    window.Show(); Pump();
                    try
                    {
                        Assert.Equal(760, window.ClientSize.Width, 0);
                        Assert.Equal(470, window.ClientSize.Height, 0);
                        Assert.Equal(new PixelPoint(120, 140), window.Position);
                        window.WindowState = WindowState.Maximized; Pump();
                        window.Hide(); Pump();
                        window.ShowFromTray(); Pump();
                        Assert.Equal(WindowState.Maximized, window.WindowState);
                        window.WindowState = WindowState.Minimized; Pump();
                        window.ShowFromTray(); Pump();
                        Assert.Equal(WindowState.Maximized, window.WindowState);
                    }
                    finally { window.Close(); Pump(); }
                }
                using (var hub = new HubController(false, store, null, null))
                {
                    var window = new MainWindow(hub);
                    window.Show(); Pump();
                    try
                    {
                        Assert.Equal(WindowState.Maximized, window.WindowState);
                        window.WindowState = WindowState.Normal; Pump();
                        Assert.Equal(760, window.ClientSize.Width, 0);
                        Assert.Equal(470, window.ClientSize.Height, 0);
                        Assert.Equal(new PixelPoint(120, 140), window.Position);
                        window.WindowState = WindowState.Minimized; Pump();
                    }
                    finally { window.Close(); Pump(); }
                }
                using (var hub = new HubController(false, store, null, null))
                {
                    var window = new MainWindow(hub);
                    window.Show(); Pump();
                    try
                    {
                        Assert.Equal(WindowState.Normal, window.WindowState);
                        Assert.Equal(760, window.ClientSize.Width, 0);
                        Assert.Equal(470, window.ClientSize.Height, 0);
                    }
                    finally { window.Close(); Pump(); }
                }
                completion.SetResult();
            }
            catch (Exception ex) { completion.SetException(ex); }
            finally { if (Directory.Exists(directory)) Directory.Delete(directory, true); }
        });
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        await completion.Task.WaitAsync(TimeSpan.FromSeconds(30));
    }

    [Fact]
    public void DisconnectedMonitorAndChangedScalingKeepPlacementOnScreen()
    {
        var saved = new WindowPlacement(4000, -2000, 2000, 1400, true);
        var fitted = WindowPlacementTracker.Fit(saved, new PixelRect(-1920, 0, 1920, 1080), 1.5);
        Assert.InRange(fitted.X, -1920, -1);
        Assert.InRange(fitted.Y, 0, 1080 - 48);
        Assert.True(fitted.X + (fitted.Width + 16) * 1.5 <= 0);
        Assert.True(fitted.Y + (fitted.Height + 48) * 1.5 <= 1080);
        Assert.True(fitted.Maximized);
        Assert.False(WindowPlacementTracker.Valid(saved with { Width = double.NaN }));
        Assert.False(WindowPlacementTracker.Valid(saved with { Height = 0 }));
    }

    private static void Pump()
    {
        using var stop = new CancellationTokenSource();
        DispatcherTimer.RunOnce(stop.Cancel, TimeSpan.FromMilliseconds(150));
        Dispatcher.UIThread.MainLoop(stop.Token);
    }
}
