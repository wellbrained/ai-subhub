using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using System.Xml.Linq;

namespace Hub.Core;

public static class MacPlatform
{
    public const string LoginLabel = "org.aisubhub.app";
    public static string Home => Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
    public static string SearchPath => string.Join(':', new[] { Path.Combine(Home, ".local", "bin"), "/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin" }) + ":" + Environment.GetEnvironmentVariable("PATH");
    public static bool UsesDefaultClaudeProfile(string directory, string home) => Path.TrimEndingDirectorySeparator(Path.GetFullPath(directory)) == Path.Combine(home, ".claude");
    public static string Quote(string value) => "'" + value.Replace("'", "'\"'\"'") + "'";
    public static string AppleScriptQuote(string value) => "\"" + value.Replace("\\", "\\\\").Replace("\"", "\\\"").Replace("\r", "\\r").Replace("\n", "\\n") + "\"";

    // The default CLI login leaves CLAUDE_CONFIG_DIR unset; explicitly setting even
    // ~/.claude creates a different Keychain namespace. Launch and read must agree.
    public static string ClaudeKeychainService(string? configDirectory) => "Claude Code-credentials" +
        (string.IsNullOrEmpty(configDirectory) ? "" : "-" + Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(configDirectory.Normalize(NormalizationForm.FormC))))[..8]);

    public static string KeychainAccount(string? user, string userName)
    {
        var name = string.IsNullOrEmpty(user) ? userName : user;
        return Regex.IsMatch(name, "^[a-zA-Z0-9._-]+$") ? name : "claude-code-user";
    }

    public static string? AppBundle(string name) => new[] { Path.Combine(Home, "Applications", name + ".app"), "/Applications/" + name + ".app" }.FirstOrDefault(Directory.Exists);

    public static ProcessStartInfo Open(string target, string? application = null)
    {
        var start = new ProcessStartInfo("/usr/bin/open") { UseShellExecute = false };
        if (application is not null) { start.ArgumentList.Add("-a"); start.ArgumentList.Add(application); }
        start.ArgumentList.Add(target);
        return start;
    }

    public static ProcessStartInfo Terminal(AccountProfile account, string directory, string executable, bool login)
    {
        var args = login ? account.Provider == ProviderKind.Claude ? " 'auth' 'login' '--claudeai'" : " 'login'" : "";
        var body = SubscriptionEnvironment.PosixScript(account) + "cd -- " + Quote(directory) + " && " + Quote(executable) + args;
        var command = "/bin/zsh -lc " + Quote(body + "; exec /bin/zsh -l");
        var start = new ProcessStartInfo("/usr/bin/osascript") { UseShellExecute = false, WorkingDirectory = directory };
        start.ArgumentList.Add("-e");
        start.ArgumentList.Add("tell application \"Terminal\"\nactivate\ndo script " + AppleScriptQuote(command) + "\nend tell");
        SubscriptionEnvironment.Apply(start, account);
        return start;
    }

    public static string LoginPlist(string executable) => "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n" +
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n" +
        new XElement("plist", new XAttribute("version", "1.0"), new XElement("dict",
            new XElement("key", "Label"), new XElement("string", LoginLabel),
            new XElement("key", "ProgramArguments"), new XElement("array", new XElement("string", executable), new XElement("string", "--tray")),
            new XElement("key", "RunAtLoad"), new XElement("true"))).ToString();

    public static void SetStartup(bool enabled, string executable)
    {
        var directory = Path.Combine(Home, "Library", "LaunchAgents");
        var path = Path.Combine(directory, LoginLabel + ".plist");
        if (!enabled) { if (File.Exists(path)) File.Delete(path); return; }
        Directory.CreateDirectory(directory);
        var temp = path + ".tmp";
        File.WriteAllText(temp, LoginPlist(executable), new UTF8Encoding(false));
        File.Move(temp, path, true);
    }
}

public static class CliProcess
{
    public static ProcessStartInfo Create(string executable, AccountProfile account, params string[] arguments)
    {
        var shim = OperatingSystem.IsWindows() && !executable.EndsWith(".exe", StringComparison.OrdinalIgnoreCase);
        var start = new ProcessStartInfo(shim ? "powershell.exe" : executable)
        { UseShellExecute = false, CreateNoWindow = true, WorkingDirectory = account.Directory };
        var script = "& " + SubscriptionEnvironment.PowerShellQuote(executable) + " " + string.Join(' ', arguments.Select(SubscriptionEnvironment.PowerShellQuote));
        foreach (var arg in shim ? ["-NoLogo", "-NoProfile", "-EncodedCommand", SubscriptionEnvironment.Encode(script)] : arguments) start.ArgumentList.Add(arg);
        SubscriptionEnvironment.Apply(start, account);
        return start;
    }
}

public static class ClaudeCredentials
{
    public static async Task<JsonDocument?> ReadAsync(AccountProfile account, CancellationToken ct)
    {
        if (OperatingSystem.IsMacOS())
        {
            var profile = MacPlatform.UsesDefaultClaudeProfile(account.Directory, MacPlatform.Home) ? null : account.Directory;
            var user = MacPlatform.KeychainAccount(Environment.GetEnvironmentVariable("USER"), Environment.UserName);
            var start = new ProcessStartInfo("/usr/bin/security") { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true };
            foreach (var arg in new[] { "find-generic-password", "-a", user, "-w", "-s", MacPlatform.ClaudeKeychainService(profile) }) start.ArgumentList.Add(arg);
            using var timeout = CancellationTokenSource.CreateLinkedTokenSource(ct);
            timeout.CancelAfter(TimeSpan.FromSeconds(8));
            using var process = Process.Start(start) ?? throw new IOException("Keychain could not be read.");
            process.ErrorDataReceived += (_, _) => { }; process.BeginErrorReadLine();
            try
            {
                var json = await process.StandardOutput.ReadToEndAsync(timeout.Token);
                await process.WaitForExitAsync(timeout.Token);
                if (process.ExitCode == 0 && !string.IsNullOrWhiteSpace(json)) return JsonDocument.Parse(json);
            }
            finally { if (!process.HasExited) process.Kill(entireProcessTree: true); }
        }
        var file = Path.Combine(account.Directory, ".credentials.json");
        return File.Exists(file) ? JsonDocument.Parse(await File.ReadAllTextAsync(file, ct)) : null;
    }
}
