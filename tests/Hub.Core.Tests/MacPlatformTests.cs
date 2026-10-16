using System.Diagnostics;
using System.Xml.Linq;
using Hub.Core;
using Xunit;

namespace Hub.Core.Tests;

public sealed class MacPlatformTests
{
    [Fact]
    public void KeychainNamespaceSeparatesDefaultFromExplicitAndNormalizesUnicode()
    {
        Assert.Equal("Claude Code-credentials", MacPlatform.ClaudeKeychainService(null));
        Assert.Equal("Claude Code-credentials-03abf0ee", MacPlatform.ClaudeKeychainService("/Users/test/.claude-work"));
        Assert.Equal(MacPlatform.ClaudeKeychainService(null), MacPlatform.ClaudeKeychainService(""));
        Assert.NotEqual(MacPlatform.ClaudeKeychainService(null), MacPlatform.ClaudeKeychainService("/Users/test/.claude"));
        Assert.NotEqual(MacPlatform.ClaudeKeychainService("/Users/test/a"), MacPlatform.ClaudeKeychainService("/Users/test/b"));
        Assert.Equal(MacPlatform.ClaudeKeychainService("/Users/test/é"), MacPlatform.ClaudeKeychainService("/Users/test/e\u0301"));
        Assert.NotEqual(MacPlatform.ClaudeKeychainService(" /Users/test/a"), MacPlatform.ClaudeKeychainService("/Users/test/a"));
    }

    [Fact]
    public void KeychainAccountMatchesCliUserFallbackAndRejectsUnsafeNames()
    {
        Assert.Equal("mac.user", MacPlatform.KeychainAccount(null, "mac.user"));
        Assert.Equal("mac.user", MacPlatform.KeychainAccount("", "mac.user"));
        Assert.Equal("work-user", MacPlatform.KeychainAccount("work-user", "mac.user"));
        Assert.Equal("claude-code-user", MacPlatform.KeychainAccount("bad user", "mac.user"));
    }

    [Fact]
    public void DefaultProfileLaunchAndCustomProfileLaunchUseMatchingNamespaces()
    {
        var defaultProfile = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Default", Path.Combine(MacPlatform.Home, ".claude"));
        Assert.Contains("unset CLAUDE_CONFIG_DIR;", SubscriptionEnvironment.PosixScript(defaultProfile));
        var other = defaultProfile with { Directory = Path.Combine(MacPlatform.Home, ".claude-other") };
        var script = SubscriptionEnvironment.PosixScript(other);
        Assert.Contains("export CLAUDE_CONFIG_DIR=" + MacPlatform.Quote(other.Directory), script);
        Assert.Contains("CLAUDE_SECURESTORAGE_CONFIG_DIR", script);
        Assert.Contains("ANTHROPIC_API_KEY", script);
    }

    [Fact]
    public void TerminalUsesLiteralPathsThroughBothShellAndAppleScript()
    {
        const string path = "/Users/test/Work's $(touch BAD); \"project\"\nnext";
        Assert.Equal("'a'\"'\"'b'", MacPlatform.Quote("a'b"));
        var account = new AccountProfile(Guid.NewGuid(), ProviderKind.Claude, "Work", path);
        var start = MacPlatform.Terminal(account, path, "/opt/tools/claude", true);
        Assert.Equal("/usr/bin/osascript", start.FileName);
        Assert.Equal(2, start.ArgumentList.Count);
        var body = SubscriptionEnvironment.PosixScript(account) + "cd -- " + MacPlatform.Quote(path) + " && '/opt/tools/claude' 'auth' 'login' '--claudeai'; exec /bin/zsh -l";
        Assert.Equal("tell application \"Terminal\"\nactivate\ndo script " + MacPlatform.AppleScriptQuote("/bin/zsh -lc " + MacPlatform.Quote(body)) + "\nend tell", start.ArgumentList[1]);
    }

    [Fact]
    public void LoginAgentStoresExecutableAndTrayArgumentAsSeparateEscapedValues()
    {
        const string executable = "/Applications/A & B <test>.app/Contents/MacOS/AI.SubHub";
        var plist = XDocument.Parse(MacPlatform.LoginPlist(executable));
        var dict = plist.Root!.Element("dict")!;
        Assert.Equal(new[] { executable, "--tray" }, dict.Element("array")!.Elements("string").Select(x => x.Value));
        Assert.NotNull(dict.Element("true"));
        Assert.Contains(MacPlatform.LoginLabel, dict.Elements("string").Select(x => x.Value));
    }

    [Fact]
    public async Task ActivationCanReconnectAndDisposeWithoutBlocking()
    {
        var name = "test-" + Guid.NewGuid();
        using var calls = new SemaphoreSlim(0);
        using var activation = new InstanceActivation(name, () => calls.Release());
        Assert.True(InstanceActivation.Signal(name));
        Assert.True(await calls.WaitAsync(TimeSpan.FromSeconds(5)));
        Assert.True(InstanceActivation.Signal(name));
        Assert.True(await calls.WaitAsync(TimeSpan.FromSeconds(5)));
    }
}
