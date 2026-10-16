using System.Diagnostics;
using System.Text;

namespace Hub.Core;

public enum TerminalTarget { WindowsTerminal, PowerShell }

public static class Executables
{
    public static string? Find(string name, string? configured = null)
    {
        if (!string.IsNullOrWhiteSpace(configured)) return File.Exists(configured) ? Path.GetFullPath(configured) : null;
        var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        var directories = new List<string> { Path.Combine(home, ".local", "bin"), Path.Combine(home, "AppData", "Roaming", "npm") };
        directories.AddRange((Environment.GetEnvironmentVariable("PATH") ?? "").Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries));
        if (name == "codex")
        {
            var bundled = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "OpenAI", "Codex", "bin");
            if (Directory.Exists(bundled)) directories.AddRange(Directory.GetDirectories(bundled).OrderDescending());
        }
        foreach (var directory in directories)
            foreach (var extension in new[] { ".exe", ".cmd", ".bat" })
            {
                var candidate = Path.Combine(directory.Trim('"'), name + extension);
                if (File.Exists(candidate)) return candidate;
            }
        return null;
    }
}

public static class SubscriptionEnvironment
{
    private static readonly string[] Overrides = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "CLAUDE_CODE_OAUTH_TOKEN", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY", "OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_API_KEY"];
    public static void Apply(ProcessStartInfo start, AccountProfile account)
    {
        foreach (var key in Overrides) start.Environment.Remove(key);
        start.Environment[account.Provider == ProviderKind.Claude ? "CLAUDE_CONFIG_DIR" : "CODEX_HOME"] = account.Directory;
    }

    public static string Script(AccountProfile account) => string.Join("; ", Overrides.Select(key => $"Remove-Item -LiteralPath 'Env:{key}' -ErrorAction SilentlyContinue")) + "; " +
        $"$env:{(account.Provider == ProviderKind.Claude ? "CLAUDE_CONFIG_DIR" : "CODEX_HOME")} = {PowerShellQuote(account.Directory)}; ";
    public static string PowerShellQuote(string value) => "'" + value.Replace("'", "''") + "'";
    public static string Encode(string script) => Convert.ToBase64String(Encoding.Unicode.GetBytes(script));
}

public sealed class LaunchService(HubSettings settings)
{
    public string? Executable(ProviderKind provider) => Executables.Find(provider == ProviderKind.Claude ? "claude" : "codex", provider == ProviderKind.Claude ? settings.ClaudeExecutable : settings.CodexExecutable);

    public void Launch(AccountProfile account, string workingDirectory, bool login = false, TerminalTarget? target = null)
    {
        var start = CreateStartInfo(account, workingDirectory, login, target);
        Directory.CreateDirectory(account.Directory);
        Process.Start(start)?.Dispose();
    }

    public ProcessStartInfo CreateStartInfo(AccountProfile account, string workingDirectory, bool login = false, TerminalTarget? target = null)
    {
        if (!Directory.Exists(workingDirectory)) throw new InvalidOperationException("The selected project folder no longer exists. Choose another folder.");
        var executable = Executable(account.Provider) ?? throw new InvalidOperationException($"{account.Provider} CLI was not found. Install it or set its executable in Settings → Tools.");
        var arguments = login ? account.Provider == ProviderKind.Claude ? " 'auth' 'login' '--claudeai'" : " 'login'" : "";
        var script = SubscriptionEnvironment.Script(account) + $"Set-Location -LiteralPath {SubscriptionEnvironment.PowerShellQuote(workingDirectory)}; & {SubscriptionEnvironment.PowerShellQuote(executable)}{arguments}";
        var terminal = (target == TerminalTarget.WindowsTerminal || target is null && settings.Terminal == "Windows Terminal") ? Executables.Find("wt") : null;
        var start = new ProcessStartInfo(terminal ?? "powershell.exe") { UseShellExecute = false, WorkingDirectory = workingDirectory };
        if (terminal is not null) start.ArgumentList.Add("new-tab");
        if (terminal is not null) start.ArgumentList.Add("powershell.exe");
        foreach (var arg in new[] { "-NoLogo", "-NoProfile", "-NoExit", "-EncodedCommand", SubscriptionEnvironment.Encode(script) }) start.ArgumentList.Add(arg);
        SubscriptionEnvironment.Apply(start, account);
        return start;
    }

    public void OpenCode(string directory)
    {
        if (!Directory.Exists(directory)) throw new InvalidOperationException("The selected project folder no longer exists.");
        var executable = Executables.Find("code") ?? throw new InvalidOperationException("VS Code was not found on PATH. Install VS Code with its PATH option enabled.");
        var script = $"& {SubscriptionEnvironment.PowerShellQuote(executable)} '--new-window' '--' {SubscriptionEnvironment.PowerShellQuote(directory)}";
        var start = new ProcessStartInfo("powershell.exe") { UseShellExecute = false, CreateNoWindow = true };
        foreach (var arg in new[] { "-NoLogo", "-NoProfile", "-EncodedCommand", SubscriptionEnvironment.Encode(script) }) start.ArgumentList.Add(arg);
        Process.Start(start)?.Dispose();
    }

    public static Uri ClaudeDesktopFolderUri(string directory) => new("claude://code/new?folder=" + Uri.EscapeDataString(Path.GetFullPath(directory)));

    public void OpenClaudeDesktopFolder(string directory)
    {
        if (!Directory.Exists(directory)) throw new InvalidOperationException("The project folder no longer exists. Edit it in Settings → Projects.");
        Process.Start(new ProcessStartInfo(ClaudeDesktopFolderUri(directory).AbsoluteUri) { UseShellExecute = true })?.Dispose();
    }
}
