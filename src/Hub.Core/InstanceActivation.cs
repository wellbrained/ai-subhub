using System.IO.Pipes;
using System.Security.Cryptography;
using System.Text;

namespace Hub.Core;

// Local current-user activation only; no network listener or account payload.
public sealed class InstanceActivation : IDisposable
{
    private readonly CancellationTokenSource _stop = new();
    private readonly string _name;
    private readonly Task _server;
    public InstanceActivation(string name, Action activated)
    {
        _name = PipeName(name);
        _server = ListenAsync(activated);
    }
    private static string PipeName(string name) => "AISubHub-" + Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile) + ":" + name)))[..24];

    public static bool Signal(string name)
    {
        try
        {
            using var client = new NamedPipeClientStream(".", PipeName(name), PipeDirection.Out, PipeOptions.CurrentUserOnly);
            client.Connect(3000); client.WriteByte(1); client.Flush(); return true;
        }
        catch (Exception ex) when (ex is IOException or TimeoutException or UnauthorizedAccessException) { return false; }
    }

    private async Task ListenAsync(Action activated)
    {
        while (!_stop.IsCancellationRequested)
        {
            try
            {
                await using var pipe = new NamedPipeServerStream(_name, PipeDirection.In, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous | PipeOptions.CurrentUserOnly);
                await pipe.WaitForConnectionAsync(_stop.Token);
                var buffer = new byte[1];
                if (await pipe.ReadAsync(buffer, _stop.Token) == 1 && buffer[0] == 1) activated();
            }
            catch (OperationCanceledException) when (_stop.IsCancellationRequested) { return; }
            catch (IOException) { /* A client closing early does not stop activation. */ }
        }
    }
    public void Dispose()
    {
        _stop.Cancel();
        try { _server.GetAwaiter().GetResult(); }
        finally { _stop.Dispose(); }
    }
}
