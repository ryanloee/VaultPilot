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
    private static readonly TimeSpan DefaultIpcTimeout = TimeSpan.FromSeconds(180);

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
    private Task? _pumpStdoutTask;
    private Task? _pumpStderrTask;
    private string? _executablePath;
    private Timer? _healthCheckTimer;
    private int _isDisposed;
    private readonly SemaphoreSlim _reconnectLock = new(1, 1);
    private int _consecutiveHealthCheckFailures;
    private volatile bool _degradedMode;
    private int _healthCheckInProgress;

    public bool IsConnected
    {
        get
        {
            try { return Volatile.Read(ref _process) is { HasExited: false }; }
            catch { return false; }
        }
    }
    public event Action<AgentStatusEvent>? AgentStatusReceived;
    public event Action<bool>? ConnectionStateChanged;

    /// <summary>
    /// Start the Rust backend process and wait for pump initialization before
    /// starting the health check timer.  Previously this fire-and-forget'd
    /// StartProcessAsync, which allowed the first health check tick (15s later)
    /// to race with a slow-starting backend and spawn a duplicate process via
    /// TryReconnectWithRetryAsync.  See #3204.
    /// </summary>
    public async Task StartAsync(string executablePath)
    {
        if (IsConnected)
        {
            return;
        }

        _executablePath = executablePath;
        await StartProcessAsync();
        StartHealthCheck();
        RegisterPowerModeHandler();
    }

    private async Task StartProcessAsync()
    {
        if (Volatile.Read(ref _isDisposed) != 0) return;
        if (string.IsNullOrWhiteSpace(_executablePath))
        {
            Trace.TraceError("StartProcessAsync: Rust backend path not set.");
            ConnectionStateChanged?.Invoke(false);
            return;
        }

        // Issue #758: Capture to local variable so the catch block can safely
        // dispose it even if DisposeAsync nulls _process concurrently.
        var proc = new Process
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
        Volatile.Write(ref _process, proc);

        try
        {
            proc.Start();
        }
        catch (Exception ex)
        {
            proc.Dispose();
            Volatile.Write(ref _process, null);
            Trace.TraceError($"StartProcessAsync: process failed to start: {ex}");
            ConnectionStateChanged?.Invoke(false);
            return;
        }

        // Issue #708: After _process.Start() succeeds, verify we haven't been
        // disposed in the gap between the initial _isDisposed check (line 73)
        // and now.  If DisposeAsync ran concurrently, the new process would
        // otherwise be orphaned with nobody to kill it.
        if (Volatile.Read(ref _isDisposed) != 0)
        {
            var orphaned = Interlocked.Exchange(ref _process, null);
            if (orphaned is not null)
            {
                try { orphaned.Kill(entireProcessTree: true); } catch { /* best effort */ }
                orphaned.Dispose();
            }
            return;
        }

        try
        {
            var oldReaderCts = Interlocked.Exchange(ref _readerCts, null);
            oldReaderCts?.Cancel();
            // Await old pump tasks to prevent concurrent readers on the same stream
            var oldStdout = _pumpStdoutTask;
            var oldStderr = _pumpStderrTask;
            if (oldStdout != null) try { await oldStdout; } catch { /* cancelled */ }
            if (oldStderr != null) try { await oldStderr; } catch { /* cancelled */ }
            oldReaderCts?.Dispose();
            _readerCts = new CancellationTokenSource();
            var token = _readerCts.Token;
            _pumpStdoutTask = Task.Run(() => PumpStdoutAsync(token));
            _pumpStderrTask = Task.Run(() => PumpStderrAsync(token));
            ConnectionStateChanged?.Invoke(true);
        }
        catch (Exception ex) when (Volatile.Read(ref _isDisposed) == 0)
        {
            Trace.TraceError($"StartProcessAsync pump setup error: {ex}");
        }
        catch
        {
            // Shutting down — swallow to prevent async void crash when _isDisposed is 1
        }
    }

    private void StartHealthCheck()
    {
        _healthCheckTimer?.Dispose();
        _healthCheckTimer = new Timer(OnHealthCheckTick, null, HealthCheckInterval, HealthCheckInterval);
    }

    private void SetHealthCheckInterval(TimeSpan interval)
    {
        try
        {
            _healthCheckTimer?.Change(interval, interval);
        }
        catch (ObjectDisposedException)
        {
            // Timer was disposed between the null-check and Change(); safe to ignore.
        }
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
        if (Volatile.Read(ref _isDisposed) != 0) return;

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
        if (Volatile.Read(ref _isDisposed) != 0) return;
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
                else if (Volatile.Read(ref _isDisposed) == 0)
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
            if (Volatile.Read(ref _isDisposed) == 0)
            {
                try
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
                catch (Exception reconnectEx)
                {
                    Trace.TraceError($"OnHealthCheckTick reconnect failed: {reconnectEx.Message}");
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
            if (Volatile.Read(ref _isDisposed) != 0 || cancellationToken.IsCancellationRequested) return false;

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
        if (Volatile.Read(ref _isDisposed) != 0) return false;

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
        if (Volatile.Read(ref _isDisposed) != 0 || _executablePath == null) return false;
        if (IsConnected && !forceRestart) return true;

        if (!await _reconnectLock.WaitAsync(TimeSpan.FromSeconds(5), cancellationToken))
        {
            return false;
        }

        try
        {
            if (Volatile.Read(ref _isDisposed) != 0 || _executablePath == null) return false;
            if (IsConnected && !forceRestart) return true;

            await DisposeProcessAsync();
            await Task.Delay(TimeSpan.FromSeconds(1), cancellationToken);

            if (Volatile.Read(ref _isDisposed) != 0) return false;

            // Issue #2721: properly await StartProcessAsync instead of
            // fire-and-forget. The old code (_ = StartProcessAsync()) could
            // silently swallow pump-setup exceptions and then check process
            // liveness after only 500ms — insufficient for slow startup
            // (disk I/O, env var resolution). Now we await the full startup
            // sequence and then poll with a reasonable timeout.
            await StartProcessAsync();

            // Poll process liveness with backoff instead of a single fixed
            // delay. StartProcessAsync completes when pumps are wired, but
            // the OS-level process may still be initializing its JSON-RPC
            // server. Give it up to 3 seconds with 100ms poll intervals.
            const int maxStartupWaitMs = 3000;
            const int pollIntervalMs = 100;
            var sw = System.Diagnostics.Stopwatch.StartNew();
            while (sw.ElapsedMilliseconds < maxStartupWaitMs)
            {
                var current = Volatile.Read(ref _process);
                if (current is { HasExited: false })
                    return true;
                if (current is { HasExited: true })
                    return false; // process crashed during startup
                await Task.Delay(pollIntervalMs, cancellationToken);
            }
            return false;
        }
        catch (OperationCanceledException)
        {
            throw; // Propagate cancellation to caller (#536)
        }
        catch
        {
            return false;
        }
        finally
        {
            try { _reconnectLock.Release(); }
            catch (ObjectDisposedException) { /* shutting down — safe to ignore */ }
        }
    }

    public async Task<T?> SendAsync<T>(string method, object? parameters, CancellationToken cancellationToken = default, TimeSpan? requestTimeout = null)
    {
        var result = await SendAsync(method, parameters, cancellationToken, requestTimeout);
        return result.ValueKind == JsonValueKind.Undefined || result.ValueKind == JsonValueKind.Null
            ? default
            : result.Deserialize<T>(_jsonOptions);
    }

    public async Task<JsonElement> SendAsync(string method, object? parameters, CancellationToken cancellationToken = default, TimeSpan? requestTimeout = null)
    {
        if (Volatile.Read(ref _isDisposed) != 0)
            throw new InvalidOperationException("Backend client disposed.");

        // Issue #750: capture _process once into a local variable to avoid
        // stale reads if DisposeProcessAsync runs between checks.
        var process = Volatile.Read(ref _process);
        if (!IsConnected || process?.StandardInput is null || process?.StandardOutput is null)
        {
            var reconnected = await EnsureConnectedAsync(cancellationToken);
            process = Volatile.Read(ref _process);
            if (!reconnected || process?.StandardInput is null || process?.StandardOutput is null)
            {
                throw new InvalidOperationException("Rust 后端尚未连接。");
            }
        }

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
            // Issue #634: wrap in try-catch — _writeLock may be disposed by
            // DisposeAsync between the _isDisposed guard above and this WaitAsync.
            try
            {
                await _writeLock.WaitAsync(cancellationToken);
            }
            catch (ObjectDisposedException)
            {
                throw new InvalidOperationException("Backend client disposed.");
            }
            try
            {
                await process.StandardInput.WriteLineAsync(payload.AsMemory(), cancellationToken);
                await process.StandardInput.FlushAsync(cancellationToken);
            }
            catch (Exception error) when (error is IOException or ObjectDisposedException or InvalidOperationException)
            {
                // #2689: Do NOT call completion.TrySetException here — the throw
                // below carries the failure to the caller, and the finally block
                // removes the entry from _pending so the TCS is never awaited.
                // Setting the exception on an un-awaited TCS triggers
                // TaskScheduler.UnobservedTaskException → spurious crash.log entries.
                _ = TryReconnectWithRetryAsync();
                throw new InvalidOperationException("Rust 后端尚未连接。", error);
            }
            finally
            {
                // Issue #653: _writeLock may be disposed by DisposeAsync between
                // WaitAsync succeeding and Release — catch ODE to avoid masking
                // the real exception with a finally-block crash.
                try { _writeLock.Release(); }
                catch (ObjectDisposedException) { /* shutting down — safe to ignore */ }
            }

            try
            {
                // Issue #710: use the caller-provided timeout (e.g. user-configured
                // RequestTimeoutMs for AI requests) instead of a hardcoded 90s limit.
                var ipcTimeout = requestTimeout ?? DefaultIpcTimeout;
                using var timeoutCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
                timeoutCts.CancelAfter(ipcTimeout);
                try
                {
                    var root = await completion.Task.WaitAsync(timeoutCts.Token);
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
                catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
                {
                    // The timeout fired (not the caller's token) — the backend
                    // likely dropped the request. Clean up and report.
                    throw new TimeoutException(
                        $"后端请求 {method} 超时（{ipcTimeout.TotalSeconds} 秒无响应），后端可能已断开。");
                }
            }
            catch (TimeoutException)
            {
                // Attempt reconnection on timeout since the backend may be dead.
                _ = TryReconnectWithRetryAsync();
                throw;
            }
        }
        finally
        {
            _pending.TryRemove(id, out _);
        }
    }

    public async ValueTask DisposeAsync()
    {
        if (Interlocked.CompareExchange(ref _isDisposed, 1, 0) != 0) return;

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

        // Cancel reader first to stop Pump thread from adding new pending entries
        var readerCts = Interlocked.Exchange(ref _readerCts, null);
        readerCts?.Cancel();

        // Then fail all existing pending requests
        FailPending("Backend client disposed.");

        await DisposeProcessAsync();

        _writeLock?.Dispose();
        _reconnectLock?.Dispose();
        readerCts?.Dispose();
    }

    private async Task DisposeProcessAsync()
    {
        // Issue #635: capture _process to local variable via atomic exchange
        // to prevent NRE from concurrent callers.
        var process = Interlocked.Exchange(ref _process, null);
        if (process is null)
        {
            return;
        }

        try
        {
            if (!process.HasExited)
            {
                process.Kill(entireProcessTree: true);
            }

            // Issue #713: WaitForExitAsync with timeout — Kill() may fail on
            // zombie/unkillable processes, which would hang disposal indefinitely.
            using var exitCts = new CancellationTokenSource(TimeSpan.FromSeconds(5));
            try { await process.WaitForExitAsync(exitCts.Token); }
            catch (OperationCanceledException) { /* timeout — proceed with dispose */ }
        }
        catch
        {
            // Ignore errors during cleanup
        }
        finally
        {
            Interlocked.Exchange(ref _readerCts, null)?.Cancel();
            process.Dispose();
            ConnectionStateChanged?.Invoke(false);
        }
    }

    private sealed record BackendRequest(string Id, string Method, object? Params);

    private async Task PumpStdoutAsync(CancellationToken token)
    {
        var process = Volatile.Read(ref _process);
        if (process?.StandardOutput is null)
        {
            return;
        }

        try
        {
            while (!token.IsCancellationRequested)
            {
                var line = await process.StandardOutput.ReadLineAsync(token);
                if (line is null)
                {
                    FailPending("Rust 后端已关闭输出通道。");
                    if (Volatile.Read(ref _isDisposed) == 0)
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
            if (Volatile.Read(ref _isDisposed) == 0)
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
        try
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
        catch (Exception ex)
        {
            System.Diagnostics.Debug.WriteLine($"HandleEvent: failed to process event: {ex.Message}");
        }
    }

    public string GetStderrTail(int maxLines = 10)
    {
        var lines = _stderrLines.ToArray();
        var start = Math.Max(0, lines.Length - maxLines);
        return string.Join(Environment.NewLine, lines[start..]);
    }

    // ─── AI Action methods (#2188) ──────────────────────────────────

    /// <summary>
    /// Execute an AI quick action (summarize, translate, explain, edit, etc.) via the backend.
    /// </summary>
    public async Task<AiActionResult> ExecuteAiActionAsync(
        AiActionType action,
        string text,
        string? targetLanguage = null,
        string? tone = null,
        string? noteId = null,
        string? instruction = null,
        string? model = null,
        CancellationToken token = default)
    {
        var parameters = new
        {
            action = action switch
            {
                AiActionType.Summarize => "summarize",
                AiActionType.Rewrite => "rewrite",
                AiActionType.Translate => "translate",
                AiActionType.Explain => "explain",
                AiActionType.ContinueWriting => "continueWriting",
                AiActionType.ExtractTodos => "extractTodos",
                AiActionType.FindRelatedNotes => "findRelatedNotes",
                AiActionType.CleanUp => "cleanUp",
                AiActionType.GenerateOutline => "generateOutline",
                AiActionType.EditNote => "editNote",
                AiActionType.SummarizeUrl => "summarizeUrl",
                AiActionType.Brainstorm => "brainstorm",
                AiActionType.ReviewNote => "reviewNote",
                AiActionType.SynthesizeWiki => "synthesizeWiki",
                AiActionType.WorkspaceQuery => "workspaceQuery",
                AiActionType.TranscribeAudio => "transcribeAudio",
                AiActionType.SuggestLinks => "suggestLinks",
                AiActionType.SynthesizeNotes => "synthesizeNotes",
                // #3362: non-exhaustive switch — future enum additions fall through
                // without a default, causing CS8509 at build and SwitchExpressionException
                // at runtime. The explicit exception preserves the diagnostic message.
                _ => throw new ArgumentOutOfRangeException(
                    nameof(action), action,
                    "Unmapped AiActionType — update ExecuteAiActionAsync switch expression")
            },
            text,
            targetLanguage,
            tone,
            noteId,
            instruction,
            modelOverride = model
        };

        var result = await SendAsync<AiActionResult>("executeAiAction", parameters, token)
            ?? throw new InvalidOperationException("后端返回了空结果。");
        return result;
    }

    /// <summary>
    /// List all available AI action types with their labels.
    /// </summary>
    public async Task<IReadOnlyList<AiActionInfo>> ListAiActionsAsync(CancellationToken token = default)
    {
        var result = await SendAsync<JsonElement>("listAiActions", new { }, token);
        if (result.ValueKind == JsonValueKind.Null || result.ValueKind == JsonValueKind.Undefined)
            return Array.Empty<AiActionInfo>();

        var actions = result.Deserialize<List<AiActionInfo>>(_jsonOptions);
        return actions is not null ? actions.AsReadOnly() : Array.Empty<AiActionInfo>();
    }

    /// <summary>
    /// Find notes related to a given note by calling the storage-backed
    /// find_related_notes_with_context endpoint.
    /// </summary>
    public async Task<IReadOnlyList<RelatedNote>?> FindRelatedNotesAsync(
        string noteId,
        int limit = 5,
        CancellationToken token = default)
    {
        return await SendAsync<IReadOnlyList<RelatedNote>>(
            "findRelatedNotes", new { id = noteId, limit }, token);
    }

    private async Task PumpStderrAsync(CancellationToken token)
    {
        var process = Volatile.Read(ref _process);
        if (process?.StandardError is null)
        {
            return;
        }

        try
        {
            while (!token.IsCancellationRequested)
            {
                var line = await process.StandardError.ReadLineAsync(token);
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
