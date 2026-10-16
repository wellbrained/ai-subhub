using System.Net;
using System.Net.Sockets;
using System.Text;

namespace Hub.Core;

/// <summary>Loopback-only liveness for CoreHub's production process control.</summary>
public sealed class ProductionHealth : IDisposable
{
    private readonly TcpListener _listener;
    private readonly CancellationTokenSource _stop = new();
    private readonly Task _worker;
    public int Port => ((IPEndPoint)_listener.LocalEndpoint).Port;

    public ProductionHealth(int port)
    {
        _listener = new TcpListener(IPAddress.Loopback, port);
        _listener.Start();
        _worker = RunAsync();
    }

    private async Task RunAsync()
    {
        try
        {
            while (!_stop.IsCancellationRequested)
            {
                using var client = await _listener.AcceptTcpClientAsync(_stop.Token);
                using var timeout = CancellationTokenSource.CreateLinkedTokenSource(_stop.Token);
                timeout.CancelAfter(TimeSpan.FromSeconds(2));
                try
                {
                    var stream = client.GetStream();
                    var buffer = new byte[1024];
                    var count = await stream.ReadAsync(buffer, timeout.Token);
                    var request = Encoding.ASCII.GetString(buffer, 0, count);
                    var healthy = request.StartsWith("GET /api/health HTTP/", StringComparison.Ordinal);
                    var body = healthy ? "{\"status\":\"ok\",\"app\":\"AI SubHub\",\"version\":\"0.13.0\"}" : "{}";
                    var response = $"HTTP/1.1 {(healthy ? "200 OK" : "404 Not Found")}\r\nContent-Type: application/json\r\nContent-Length: {Encoding.UTF8.GetByteCount(body)}\r\nConnection: close\r\n\r\n{body}";
                    await stream.WriteAsync(Encoding.UTF8.GetBytes(response), timeout.Token);
                }
                catch (Exception ex) when (ex is IOException or OperationCanceledException or SocketException) { }
            }
        }
        catch (Exception ex) when (ex is OperationCanceledException or SocketException or ObjectDisposedException) { }
    }

    public void Dispose()
    {
        _stop.Cancel(); _listener.Stop();
        _worker.GetAwaiter().GetResult(); _stop.Dispose();
    }
}
