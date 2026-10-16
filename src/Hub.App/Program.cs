using Avalonia;
using Hub.Core;

namespace Hub.App;

internal static class Program
{
    private static Action? _activate;
    private static int _activationRequested;
    public static bool ActivationRequested => Volatile.Read(ref _activationRequested) != 0;
    public static bool CoreHubManaged { get; private set; }
    [STAThread]
    public static int Main(string[] args)
    {
        if (args.Contains("--smoke-test")) return SmokeTest.Run(args).GetAwaiter().GetResult();
        if (!OperatingSystem.IsWindows() && !OperatingSystem.IsMacOS()) return 2;
        var name = args.Contains("--demo") ? "AISubHub-Demo" : "AISubHub";
        using var instance = new Mutex(true, OperatingSystem.IsWindows() ? @"Local\" + name : name, out var first);
        if (!first)
        {
            if (InstanceActivation.Signal(name)) return 0;
            // Activate a still-running Windows 0.10 instance during an upgrade.
            if (OperatingSystem.IsWindows() && EventWaitHandle.TryOpenExisting(@"Local\" + name + "-Show", out var legacy))
            {
                using (legacy) legacy.Set();
                return 0;
            }
            return 3;
        }
        try
        {
            using var activation = new InstanceActivation(name, () =>
            {
                Interlocked.Exchange(ref _activationRequested, 1);
                Volatile.Read(ref _activate)?.Invoke();
            });
            var healthIndex = Array.IndexOf(args, "--health-port");
            var port = 0;
            if (healthIndex >= 0 && (healthIndex + 1 >= args.Length || !int.TryParse(args[healthIndex + 1], out port) || port is < 1024 or > 65535)) return 2;
            CoreHubManaged = healthIndex >= 0;
            using var health = CoreHubManaged ? new ProductionHealth(port) : null;
            BuildAvaloniaApp().StartWithClassicDesktopLifetime(args); return 0;
        }
        finally { instance.ReleaseMutex(); }
    }

    public static void OnActivation(Action action)
    {
        Volatile.Write(ref _activate, action);
        if (ActivationRequested) action();
    }

    public static AppBuilder BuildAvaloniaApp() => AppBuilder.Configure<App>().UsePlatformDetect();
}
