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

    public bool IsConnected => _process is { HasExited: false };
    public event Action<AgentStatusEvent>? AgentStatusReceived;

    public void Start(string executablePath)
    {
        if (IsConnected)
        {
            return;
        }

        _process = new Process
        {
            StartInfo = new ProcessStartInfo
            {
                FileName = executablePath,
                WorkingDirectory = Path.GetDirectoryName(executablePath) ?? AppContext.BaseDirectory,
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
                await _process.StandardInput.WriteLineAsync(payload.AsMemory(), cancellationToken);
                await _process.StandardInput.FlushAsync(cancellationToken);
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
        finally
        {
            _readerCts?.Cancel();
            _process.Dispose();
            _process = null;
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
