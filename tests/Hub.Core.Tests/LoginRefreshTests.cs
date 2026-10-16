using System.Diagnostics;
using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class LoginRefreshTests
{
    [Fact]
    public async Task SuccessfulWindowsLoginSignalsHubWithoutClosingTerminal()
    {
        if (!OperatingSystem.IsWindows()) return;
        var directory = Path.Combine(Path.GetTempPath(), "hub-login-test-" + Guid.NewGuid());
        Directory.CreateDirectory(directory);
        try
        {
            var loggedIn = Path.Combine(directory, "synthetic-login-succeeded");
            var completion = Path.Combine(directory, "completion.txt");
            var cli = Path.Combine(directory, "fake-claude.cmd");
            await File.WriteAllTextAsync(cli, "@echo off\r\necho synthetic-login>\"" + loggedIn + "\"\r\nexit /b 0\r\n");
            var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Synthetic", directory);
            var launcher = new LaunchService(new HubSettings { ClaudeExecutable = cli });
            var start = launcher.CreateStartInfo(account, directory, login: true, target: TerminalTarget.PowerShell, completionFile: completion);
            Assert.Contains("-NoExit", start.ArgumentList);
            // Run the real generated terminal command headlessly; retaining the
            // interactive terminal itself is intentionally outside this harness.
            start.ArgumentList.Remove("-NoExit");
            start.CreateNoWindow = true; start.RedirectStandardError = true; start.RedirectStandardOutput = true;
            using var process = Process.Start(start)!;
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(15));
            var stderr = process.StandardError.ReadToEndAsync(timeout.Token);
            var stdout = process.StandardOutput.ReadToEndAsync(timeout.Token);
            await process.WaitForExitAsync(timeout.Token);
            await Task.WhenAll(stderr, stdout);
            Assert.True(File.Exists(loggedIn), "Synthetic CLI did not complete login.");
            Assert.True(File.Exists(completion), "CLI login succeeded but Hub never received completion, so usage cannot refresh.");
            Assert.Equal("0", (await File.ReadAllTextAsync(completion)).Trim());
        }
        finally { Directory.Delete(directory, true); }
    }
}
