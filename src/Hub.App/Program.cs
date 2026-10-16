using Avalonia;
using Hub.Core;

namespace Hub.App;

internal static class Program
{
    private static Mutex? _instance;
    private static EventWaitHandle? _activation;
    private static readonly ManualResetEvent ActivationStop = new(false);
    private static Task? _activationTask;
    private static int _activationRequested;
    public static bool ActivationRequested => Volatile.Read(ref _activationRequested) != 0;
    public static bool CoreHubManaged { get; private set; }
    [STAThread]
    public static int Main(string[] args)
    {
        if (args.Contains("--smoke-test")) return SmokeTest.Run(args).GetAwaiter().GetResult();
        if (!OperatingSystem.IsWindows()) return 2;
        var instanceName = args.Contains("--demo") ? @"Local\AISubscriptionHub-Demo" : @"Local\AISubscriptionHub";
        _activation = new EventWaitHandle(false, EventResetMode.AutoReset, instanceName + "-Show");
        _instance = new Mutex(true, instanceName, out var first);
        if (!first) { _activation.Set(); _activation.Dispose(); _instance.Dispose(); return 0; }
        var healthIndex = Array.IndexOf(args, "--health-port");
        try
        {
            var port = 0;
            if (healthIndex >= 0 && (healthIndex + 1 >= args.Length || !int.TryParse(args[healthIndex + 1], out port) || port is < 1024 or > 65535)) return 2;
            CoreHubManaged = healthIndex >= 0;
            using var health = CoreHubManaged ? new ProductionHealth(port) : null;
            BuildAvaloniaApp().StartWithClassicDesktopLifetime(args); return 0;
        }
        finally
        {
            ActivationStop.Set(); _activationTask?.Wait(TimeSpan.FromSeconds(1));
            _instance.ReleaseMutex(); _instance.Dispose(); _activation.Dispose(); ActivationStop.Dispose();
        }
    }

    public static void OnActivation(Action action)
    {
        var signal = _activation;
        if (signal is null) return;
        _activationTask = Task.Factory.StartNew(() =>
        {
            while (WaitHandle.WaitAny([signal, ActivationStop]) == 0)
            {
                Interlocked.Exchange(ref _activationRequested, 1);
                action();
            }
        }, CancellationToken.None, TaskCreationOptions.LongRunning, TaskScheduler.Default);
    }

    public static AppBuilder BuildAvaloniaApp() => AppBuilder.Configure<App>().UsePlatformDetect();
}
