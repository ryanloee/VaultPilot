using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using VaultPilot.WinUI.Models;
using System.Threading;
using System.Collections.Concurrent;

namespace VaultPilot.WinUI.Backend;

public sealed class BackendClient : IAsyncDisposable
{
    private static readonly UTF8Encoding Utf8NoBom = new(encoderShouldEmitUTF8Identifier: false);
    private static readonly TimeSpan HealthCheckInterval = TimeSpan.FromSeconds(30);
    private static readonly TimeSpan ReconnectDelay = TimeSpan.FromSeconds(2);

    private readonly JsonSerializerOptions _jsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        PropertyNameCaseInsensitive = true,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull
    };

    private Process? _process;
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private readonly ConcurrentDictionary<string, TaskCompletionSource<JsonElement>> _pending = new();
    private CancellationTokenSource? _readerCts;
    private string? _executablePath;
    private Timer? _healthCheckTimer;
    private bool _isDisposed;
    private readonly SemaphoreSlim _reconnectLock = new(1, 1);

    public bool IsConnected => _process is { HasExited: false };
    public event Action<AgentStatusEvent>? AgentStatusReceived;
    public event Action<bool>? ConnectionStateChanged;

    public void Start(string executablePath)
    {
        if (IsConnected)
        {
            return;
        }

        _executablePath = executablePath;
        StartProcess();
        StartHealthCheck();
    }

    private void StartProcess()
    {
        if (_isDisposed) return;
        if (string.IsNullOrWhiteSpace(_executablePath))
        {
            throw new InvalidOperationException("Rust 后端路径尚未设置。");
        }

        _process = new Process
        {
            StartInfo = new ProcessStartInfo
            {
                FileName = _executablePath,
                WorkingDirectory = Path.GetDirectoryName(_executablePath) ?? AppContext.BaseDirectory,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = false,
                StandardInputEncoding = Utf8NoBom,
                StandardOutputEncoding = Utf8NoBom,
                UseShellExecute = false,
                CreateNoWindow = true
            }
        };

        _process.Start();
        _readerCts = new CancellationTokenSource();
        _ = Task.Run(() => PumpStdoutAsync(_readerCts.Token));
        ConnectionStateChanged?.Invoke(true);
    }

    private void StartHealthCheck()
    {
        _healthCheckTimer?.Dispose();
        _healthCheckTimer = new Timer(OnHealthCheckTick, null, HealthCheckInterval, HealthCheckInterval);
    }

    private async void OnHealthCheckTick(object? state)
    {
        if (_isDisposed || !IsConnected)
        {
            if (!_isDisposed && _executablePath != null)
            {
                await TryReconnectAsync();
            }
            return;
        }

        try
        {
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(5));
            await SendAsync("ping", new { }, cts.Token);
        }
        catch
        {
            // Ping failed, connection may be dead
            if (!_isDisposed)
            {
                await TryReconnectAsync(forceRestart: true);
            }
        }
    }

    public Task<bool> EnsureConnectedAsync(CancellationToken cancellationToken = default)
    {
        return IsConnected
            ? Task.FromResult(true)
            : TryReconnectAsync(cancellationToken);
    }

    private async Task<bool> TryReconnectAsync(
        CancellationToken cancellationToken = default,
        bool forceRestart = false)
    {
        if (_isDisposed || _executablePath == null) return false;
        if (IsConnected && !forceRestart) return true;

        await _reconnectLock.WaitAsync(cancellationToken);
        try
        {
            if (_isDisposed || _executablePath == null) return false;
            if (IsConnected && !forceRestart) return true;

            await DisposeProcessAsync();
            await Task.Delay(ReconnectDelay, cancellationToken);

            if (!_isDisposed)
            {
                StartProcess();
                return true;
            }
        }
        catch
        {
            // Reconnect failed; health checks and future requests can retry.
        }
        finally
        {
            _reconnectLock.Release();
        }

        return false;
    }

    public async Task<T?> SendAsync<T>(string method, object? parameters, CancellationToken cancellationToken = default)
    {
        var result = await SendAsync(method, parameters, cancellationToken);
        return result.ValueKind == JsonValueKind.Undefined
            ? default
            : result.Deserialize<T>(_jsonOptions);
    }

    public async Task<JsonElement> SendAsync(string method, object? parameters, CancellationToken cancellationToken = default)
    {
        if (!IsConnected || _process?.StandardInput is null || _process.StandardOutput is null)
        {
            var reconnected = await EnsureConnectedAsync(cancellationToken);
            if (!reconnected || _process?.StandardInput is null || _process.StandardOutput is null)
            {
                throw new InvalidOperationException("Rust 后端尚未连接。");
            }
        }

        var process = _process;
        if (process is null)
        {
            throw new InvalidOperationException("Rust 后端尚未连接。");
        }

        var id = Guid.NewGuid().ToString("N");
        var request = new BackendRequest(id, method, parameters);
        var payload = JsonSerializer.Serialize(request, _jsonOptions);
        var completion = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        if (!_pending.TryAdd(id, completion))
        {
            throw new InvalidOperationException("后端请求 ID 冲突。");
        }

        try
        {
            await _writeLock.WaitAsync(cancellationToken);
            try
            {
                await process.StandardInput.WriteLineAsync(payload.AsMemory(), cancellationToken);
                await process.StandardInput.FlushAsync(cancellationToken);
            }
            catch (Exception error) when (error is IOException or ObjectDisposedException or InvalidOperationException)
            {
                completion.TrySetException(new InvalidOperationException("Rust 后端尚未连接。", error));
                _ = TryReconnectAsync(CancellationToken.None, forceRestart: true);
                throw new InvalidOperationException("Rust 后端尚未连接。", error);
            }
            finally
            {
                _writeLock.Release();
            }

            var root = await completion.Task.WaitAsync(cancellationToken);
            if (root.TryGetProperty("error", out var error))
            {
                var message = error.TryGetProperty("message", out var messageElement)
                    ? messageElement.GetString()
                    : "后端请求失败。";
                throw new InvalidOperationException(message);
            }

            return root.TryGetProperty("result", out var result)
                ? result.Clone()
                : default;
        }
        finally
        {
            _pending.TryRemove(id, out _);
        }
    }

    public async ValueTask DisposeAsync()
    {
        if (_isDisposed) return;
        _isDisposed = true;

        _healthCheckTimer?.Dispose();
        _healthCheckTimer = null;

        await DisposeProcessAsync();
    }

    private async Task DisposeProcessAsync()
    {
        if (_process is null)
        {
            return;
        }

        try
        {
            if (!_process.HasExited)
            {
                _process.Kill(entireProcessTree: true);
            }

            await _process.WaitForExitAsync();
        }
        catch
        {
            // Ignore errors during cleanup
        }
        finally
        {
            _readerCts?.Cancel();
            _process.Dispose();
            _process = null;
            ConnectionStateChanged?.Invoke(false);
        }
    }

    private sealed record BackendRequest(string Id, string Method, object? Params);

    private async Task PumpStdoutAsync(CancellationToken token)
    {
        if (_process?.StandardOutput is null)
        {
            return;
        }

        try
        {
            while (!token.IsCancellationRequested)
            {
                var line = await _process.StandardOutput.ReadLineAsync(token);
                if (line is null)
                {
                    FailPending("Rust 后端已关闭输出通道。");
                    // Trigger reconnect on next health check
                    if (!_isDisposed)
                    {
                        _ = TryReconnectAsync(CancellationToken.None, forceRestart: true);
                    }
                    return;
                }

                if (string.IsNullOrWhiteSpace(line))
                {
                    continue;
                }

                JsonDocument document;
                try
                {
                    document = JsonDocument.Parse(line);
                }
                catch (JsonException)
                {
                    continue;
                }

                using (document)
                {
                    var root = document.RootElement;
                    if (!root.TryGetProperty("id", out var responseId))
                    {
                        HandleEvent(root);
                        continue;
                    }

                    var id = responseId.GetString();
                    if (id is not null && _pending.TryGetValue(id, out var completion))
                    {
                        completion.TrySetResult(root.Clone());
                    }
                }
            }
        }
        catch (OperationCanceledException)
        {
        }
        catch (Exception error)
        {
            FailPending($"后端响应读取失败：{error.Message}");
            // Trigger reconnect on next health check
            if (!_isDisposed)
            {
                _ = TryReconnectAsync(CancellationToken.None, forceRestart: true);
            }
        }
    }

    private void FailPending(string message)
    {
        foreach (var completion in _pending.Values)
        {
            completion.TrySetException(new InvalidOperationException(message));
        }
    }

    private void HandleEvent(JsonElement root)
    {
        if (!root.TryGetProperty("event", out var eventElement)
            || eventElement.GetString() != "agentStatus"
            || !root.TryGetProperty("payload", out var payload))
        {
            return;
        }

        var status = payload.Deserialize<AgentStatusEvent>(_jsonOptions);
        if (status is not null)
        {
            AgentStatusReceived?.Invoke(status);
        }
    }

    public string GetStderrTail(int maxLines = 10)
    {
        return string.Empty;
    }
}
