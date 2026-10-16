namespace Hub.Core;

// Terminal processes can stay open or be detached from their launcher. A unique,
// credential-free exit-code file reports the CLI result independently of either.
public sealed class LoginCompletion : IDisposable
{
    private readonly string _directory;
    public string FilePath { get; }
    public LoginCompletion()
    {
        _directory = Path.Combine(Path.GetTempPath(), "AI-SubHub-login-" + Guid.NewGuid().ToString("N"));
        if (OperatingSystem.IsWindows()) Directory.CreateDirectory(_directory);
        else Directory.CreateDirectory(_directory, UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute);
        FilePath = Path.Combine(_directory, "exit-code");
    }

    public async Task<int?> WaitAsync(CancellationToken ct, TimeSpan? timeout = null)
    {
        var deadline = DateTimeOffset.UtcNow + (timeout ?? TimeSpan.FromMinutes(10));
        while (DateTimeOffset.UtcNow < deadline)
        {
            ct.ThrowIfCancellationRequested();
            if (File.Exists(FilePath))
            {
                try
                {
                    if (int.TryParse(await File.ReadAllTextAsync(FilePath, ct), out var exitCode)) return exitCode;
                }
                catch (IOException) { /* Retry a file still being written by the terminal. */ }
            }
            await Task.Delay(TimeSpan.FromMilliseconds(200), ct);
        }
        return null;
    }

    public void Dispose()
    {
        try { File.Delete(FilePath); Directory.Delete(_directory); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { /* Best-effort removal of this session's signal only. */ }
    }
}
