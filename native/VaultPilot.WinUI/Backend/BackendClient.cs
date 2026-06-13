using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.Win32;
using VaultPilot.WinUI.Models;
using System.Threading;
using System.Collections.Concurrent;

namespace VaultPilot.WinUI.Backend;

public sealed class BackendClient : IAsyncDisposable
{
    private static readonly UTF8Encoding Utf8NoBom = new(encoderShouldEmitUTF8Identifier: false);
    private static readonly TimeSpan HealthCheckInterval = TimeSpan.FromSeconds(15);
    private static readonly TimeSpan DegradedHealthCheckInterval = TimeSpan.FromSeconds(60);
    private static readonly TimeSpan PingTimeout = TimeSpan.FromSeconds(30);
    private const int MaxReconnectAttempts = 6;
    private const int MaxStderrLines = 50;
    private const int DegradedFailureThreshold = 3; // consecutive health check cycles before switching to degraded mode
    private static readonly TimeSpan MinBackoff = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan MaxBackoff = TimeSpan.FromSeconds(60);

    private readonly ConcurrentQueue<string> _stderrLines = new();

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
    private volatile bool _isDisposed;
    private readonly SemaphoreSlim _reconnectLock = new(1, 1);
    private int _consecutiveHealthCheckFailures;
    private volatile bool _degradedMode;
    private int _healthCheckInProgress;

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
        RegisterPowerModeHandler();
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
                RedirectStandardError = true,
                StandardInputEncoding = Utf8NoBom,
                StandardOutputEncoding = Utf8NoBom,
                UseShellExecute = false,
                CreateNoWindow = true
            }
        };

        _process.Start();
        _readerCts?.Cancel();
        _readerCts?.Dispose();
        _readerCts = new CancellationTokenSource();
        _ = Task.Run(() => PumpStdoutAsync(_readerCts.Token));
        _ = Task.Run(() => PumpStderrAsync(_readerCts.Token));
        ConnectionStateChanged?.Invoke(true);
    }

    private void StartHealthCheck()
    {
        _healthCheckTimer?.Dispose();
        _healthCheckTimer = new Timer(OnHealthCheckTick, null, HealthCheckInterval, HealthCheckInterval);
    }

    private void SetHealthCheckInterval(TimeSpan interval)
    {
        _healthCheckTimer?.Change(interval, interval);
    }

    private void RegisterPowerModeHandler()
    {
        try
        {
            SystemEvents.PowerModeChanged += OnPowerModeChanged;
        }
        catch
        {
            // SystemEvents may not be available in all contexts; ignore
        }
    }

    private async void OnPowerModeChanged(object sender, PowerModeChangedEventArgs e)
    {
        if (_isDisposed) return;

        try
        {
            if (e.Mode == PowerModes.Resume)
            {
                // System just woke up — proactively trigger reconnection
                Interlocked.Exchange(ref _consecutiveHealthCheckFailures, 0);
                _degradedMode = false;
                SetHealthCheckInterval(HealthCheckInterval);

                if (!IsConnected)
                {
                    _ = TryReconnectWithRetryAsync();
                }
            }
        }
        catch (Exception ex)
        {
            Trace.TraceError($"OnPowerModeChanged error: {ex}");
        }
    }

    private async void OnHealthCheckTick(object? state)
    {
        if (_isDisposed) return;
        if (Interlocked.CompareExchange(ref _healthCheckInProgress, 1, 0) != 0) return;

        try
        {
            if (!IsConnected)
            {
                var reconnected = await TryReconnectWithRetryAsync();
                if (reconnected)
                {
                    Interlocked.Exchange(ref _consecutiveHealthCheckFailures, 0);
                    _degradedMode = false;
                    SetHealthCheckInterval(HealthCheckInterval);
                }
                else if (!_isDisposed)
                {
                    OnConsecutiveHealthCheckFailure();
                }
                return;
            }

            using var cts = new CancellationTokenSource(PingTimeout);
            await SendAsync("ping", new { }, cts.Token);

            // Ping succeeded — reset failure tracking
            if (Volatile.Read(ref _consecutiveHealthCheckFailures) > 0)
            {
                Interlocked.Exchange(ref _consecutiveHealthCheckFailures, 0);
                _degradedMode = false;
                SetHealthCheckInterval(HealthCheckInterval);
            }
        }
        catch
        {
            if (!_isDisposed)
            {
                var reconnected = await TryReconnectWithRetryAsync();
                if (reconnected)
                {
                    Interlocked.Exchange(ref _consecutiveHealthCheckFailures, 0);
                    _degradedMode = false;
                    SetHealthCheckInterval(HealthCheckInterval);
                }
                else
                {
                    OnConsecutiveHealthCheckFailure();
                }
            }
        }
        finally
        {
            Interlocked.Exchange(ref _healthCheckInProgress, 0);
        }
    }

    private void OnConsecutiveHealthCheckFailure()
    {
        var failures = Interlocked.Increment(ref _consecutiveHealthCheckFailures);
        ConnectionStateChanged?.Invoke(false);

        // After repeated failures, switch to degraded mode with slower health checks
        if (failures >= DegradedFailureThreshold && !_degradedMode)
        {
            _degradedMode = true;
            SetHealthCheckInterval(DegradedHealthCheckInterval);
        }
    }

    private static TimeSpan GetBackoffDelay(int attempt)
    {
        // Exponential backoff: 5s, 10s, 20s, 40s, 60s, 60s...
        var delay = TimeSpan.FromTicks(MinBackoff.Ticks * (1L << (attempt - 1)));
        return delay > MaxBackoff ? MaxBackoff : delay;
    }

    private async Task<bool> TryReconnectWithRetryAsync(CancellationToken cancellationToken = default)
    {
        for (int attempt = 1; attempt <= MaxReconnectAttempts; attempt++)
        {
            if (_isDisposed || cancellationToken.IsCancellationRequested) return false;

            var success = await TryReconnectAsync(forceRestart: true, cancellationToken: cancellationToken);
            if (success)
            {
                try
                {
                    // Give the process more time to initialize after restart
                    await Task.Delay(TimeSpan.FromSeconds(2), cancellationToken);
                    using var timeoutCts = new CancellationTokenSource(PingTimeout);
                    using var linkedCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, timeoutCts.Token);
                    await SendAsync("ping", new { }, linkedCts.Token);
                    ConnectionStateChanged?.Invoke(true);
                    return true;
                }
                catch
                {
                    // Process started but not responding
                }
            }

            if (attempt < MaxReconnectAttempts)
            {
                var backoff = GetBackoffDelay(attempt);
                await Task.Delay(backoff, cancellationToken);
            }
        }

        return false;
    }

    /// <summary>
    /// Manually trigger a reconnection attempt. Call this from a UI "Reconnect" button.
    /// </summary>
    public async Task<bool> ReconnectAsync(CancellationToken cancellationToken = default)
    {
        if (_isDisposed) return false;

        // Reset failure tracking so health check interval goes back to normal
        Interlocked.Exchange(ref _consecutiveHealthCheckFailures, 0);
        _degradedMode = false;
        SetHealthCheckInterval(HealthCheckInterval);

        return await TryReconnectWithRetryAsync(cancellationToken);
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

        if (!await _reconnectLock.WaitAsync(TimeSpan.FromSeconds(5), cancellationToken))
        {
            return false;
        }

        try
        {
            if (_isDisposed || _executablePath == null) return false;
            if (IsConnected && !forceRestart) return true;

            await DisposeProcessAsync();
            await Task.Delay(TimeSpan.FromSeconds(1), cancellationToken);

            if (_isDisposed) return false;

            StartProcess();

            // Verify process actually started
            await Task.Delay(500, cancellationToken);
            return _process is { HasExited: false };
        }
        catch
        {
            return false;
        }
        finally
        {
            _reconnectLock.Release();
        }
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
                _ = TryReconnectWithRetryAsync();
                throw new InvalidOperationException("Rust 后端尚未连接。", error);
            }
            finally
            {
                _writeLock.Release();
            }

            var root = await completion.Task.WaitAsync(cancellationToken);
            if (root.TryGetProperty("error", out var errorProp))
            {
                var message = errorProp.TryGetProperty("message", out var messageElement)
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

        try
        {
            SystemEvents.PowerModeChanged -= OnPowerModeChanged;
        }
        catch
        {
            // Ignore if SystemEvents is not available
        }

        _healthCheckTimer?.Dispose();
        _healthCheckTimer = null;

        await DisposeProcessAsync();

        _writeLock?.Dispose();
        _reconnectLock?.Dispose();
        _readerCts?.Dispose();
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
                    if (!_isDisposed)
                    {
                        _ = TryReconnectWithRetryAsync();
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
            if (!_isDisposed)
            {
                _ = TryReconnectWithRetryAsync();
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
        var lines = _stderrLines.ToArray();
        var start = Math.Max(0, lines.Length - maxLines);
        return string.Join(Environment.NewLine, lines[start..]);
    }

    private async Task PumpStderrAsync(CancellationToken token)
    {
        if (_process?.StandardError is null)
        {
            return;
        }

        try
        {
            while (!token.IsCancellationRequested)
            {
                var line = await _process.StandardError.ReadLineAsync(token);
                if (line is null)
                {
                    return;
                }

                _stderrLines.Enqueue(line);

                // Trim the buffer to keep only the most recent lines.
                while (_stderrLines.Count > MaxStderrLines)
                {
                    _stderrLines.TryDequeue(out _);
                }
            }
        }
        catch (OperationCanceledException)
        {
        }
        catch
        {
            // Swallow stderr read errors — they should not crash the client.
        }
    }
}
