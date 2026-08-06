using VaultPilot.WinUI.Models;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Linq;
using System.Threading;

namespace VaultPilot.WinUI;

/// <summary>
/// Chat session lifecycle — loading, creating, deleting, switching, compression,
/// turn management, and session list — split from MainWindow.Chat.cs (#1344).
/// </summary>
public sealed partial class MainWindow : Window
{
    // ── Chat constants ──
    private const double ContextCompressionThreshold = 0.95;
    private const int RecentTurnsAfterCompression = 8;
    // #2834: hard cap on retained in-memory turns per session. Context
    // compression prunes old turns only when the projected token count nears
    // the model's context window; with a large window a very long session could
    // otherwise accumulate unbounded turns (and their attachments) in memory.
    private const int MaxTurnsPerSession = 512;
    private const ulong ImageAttachmentTokenEstimate = 1200;
    private const string MarkdownOpenTag = "<vp-markdown>";
    private const string MarkdownCloseTag = "</vp-markdown>";

    // ── Chat fields ──
    private ChatState _chatState = new(string.Empty, Array.Empty<ChatSession>());
    private readonly SemaphoreSlim _chatStateLock = new(1, 1);
    private string _currentSessionId = string.Empty;
    private readonly List<ChatAttachment> _attachments = [];
    private double _contextUsagePercent;
    private FrameworkElement? _thinkingIndicator;
    private CancellationTokenSource? _activeRequestCts;
    private volatile Task? _activeRequestTask;
    private int _requestInProgress; // #676: guard against concurrent ExecuteAiRequestAsync calls
    private GroundedAnswer? _lastAiAnswer;

    // #3508: Incremental render tracking — avoids full O(n) panel rebuild on
    // every message send. RenderCurrentSession updates these after a full
    // rebuild; AppendNewTurns uses them to append only newly-added turns.
    private string? _lastRenderedSessionId;
    private int _lastRenderedTurnCount;

    // ── Chat session loading ──

    private async Task<ChatState> TryLoadChatStateAsync()
    {
        try
        {
            return await SendWithTimeoutAsync(
                (token) => _backendClient.SendAsync<ChatState>("loadChatState", new { }, token),
                "loadChatState")
                ?? new ChatState(string.Empty, Array.Empty<ChatSession>());
        }
        catch (Exception error)
        {
            AddSystemMessage("错误", $"聊天记录读取失败，已使用空会话：{LocalizeError(error.Message)}");
            return new ChatState(string.Empty, Array.Empty<ChatSession>());
        }
    }

    // ── Session lifecycle ──

    private void OnSessionSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (SessionList.SelectedItem is not SessionListItem item)
        {
            return;
        }

        if (item.Id == _currentSessionId)
        {
            return;
        }

        _currentSessionId = item.Id;
        RenderCurrentSession();
    }

    private async void OnDeleteSessionClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            var session = CurrentSession();
            if (session is null)
            {
                return;
            }

            var dialog = new ContentDialog
            {
                XamlRoot = RootGrid.XamlRoot,
                Title = "删除会话",
                Content = $"确认删除「{session.Title}」吗？此操作不可撤销。",
                PrimaryButtonText = "删除",
                CloseButtonText = "取消",
                DefaultButton = ContentDialogButton.Close
            };

            if (await dialog.ShowAsync() != ContentDialogResult.Primary)
            {
                return;
            }

            await _chatStateLock.WaitAsync();
            try
            {
                var remaining = _chatState.Sessions
                    .Where(item => item.Id != session.Id)
                    .ToArray();

                _chatState = new ChatState(
                    remaining.FirstOrDefault()?.Id ?? string.Empty,
                    remaining);
                _currentSessionId = _chatState.CurrentSessionId;
            }
            finally
            {
                _chatStateLock.Release();
            }
            EnsureCurrentSession();
            await SaveChatStateAsync();
            RefreshSessions();
            RenderCurrentSession();

            UpdateStatusBar("success", "会话已删除", $"已删除「{session.Title}」。");
        }
        catch (Exception error)
        {
            ShowError("删除会话失败", error);
        }
    }

    private async void OnNewSessionClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            var now = DateTimeOffset.UtcNow.ToString("O");
            var session = new ChatSession(
                Guid.NewGuid().ToString("N"),
                "新对话",
                Array.Empty<ChatTurn>(),
                null,
                now,
                now);

            await _chatStateLock.WaitAsync();
            try
            {
                _chatState = new ChatState(
                    session.Id,
                    [session, .. _chatState.Sessions]);
                _currentSessionId = session.Id;
            }
            finally
            {
                _chatStateLock.Release();
            }
            EnsureCurrentSession();
            await SaveChatStateAsync();
            RefreshSessions();
            RenderCurrentSession();
            ScrollToLatest();

            UpdateStatusBar("success", "新对话", "已创建新对话。");
        }
        catch (Exception error)
        {
            ShowError("新建会话失败", error);
        }
    }

    private void EnsureCurrentSession()
    {
        if (_chatState.Sessions.Count > 0)
        {
            _currentSessionId = string.IsNullOrWhiteSpace(_chatState.CurrentSessionId)
                ? _chatState.Sessions[0].Id
                : _chatState.CurrentSessionId;
            return;
        }

        var now = DateTimeOffset.UtcNow.ToString("O");
        _currentSessionId = Guid.NewGuid().ToString("N");
        _chatState = new ChatState(
            _currentSessionId,
            new[]
            {
                new ChatSession(
                    _currentSessionId,
                    "新对话",
                    Array.Empty<ChatTurn>(),
                    null,
                    now,
                    now)
            });
    }

    // ── Chat session queries ──

    private ChatSession? CurrentSession()
    {
        return _chatState.Sessions.FirstOrDefault(session => session.Id == _currentSessionId)
            ?? _chatState.Sessions.FirstOrDefault();
    }

    private ChatSession? FindSessionById(string sessionId)
    {
        return _chatState.Sessions.FirstOrDefault(session => session.Id == sessionId);
    }

    private ConversationTurn[] GetConversationHistory(string sessionId)
    {
        var session = FindSessionById(sessionId) ?? CurrentSession();
        if (session is null)
        {
            return Array.Empty<ConversationTurn>();
        }

        var history = new List<ConversationTurn>();
        if (!string.IsNullOrWhiteSpace(session.Summary?.Text))
        {
            history.Add(new ConversationTurn("system", $"此前对话摘要：{session.Summary.Text}"));
        }

        history.AddRange(session.Turns
            .Where(turn => !string.IsNullOrWhiteSpace(turn.Text))
            .Select(turn => new ConversationTurn(turn.Role, turn.Text)));
        return history.ToArray();
    }

    // ── Context compression ──

    private async Task CompressCurrentSessionIfNeededAsync(string sessionId,
        string pendingText,
        IReadOnlyList<ChatAttachment> pendingAttachments,
        CancellationToken cancellationToken = default)
    {
        var session = FindSessionById(sessionId) ?? CurrentSession();
        if (session is null)
        {
            return;
        }

        var contextWindow = ResolveContextWindowTokens();
        var projectedTokens = EstimateSessionTokensCached(session) + EstimateTurnTokens(pendingText, pendingAttachments);
        if (contextWindow == 0 || projectedTokens < (ulong)(contextWindow * ContextCompressionThreshold))
        {
            return;
        }

        var compressibleCount = Math.Max(0, session.Turns.Count - RecentTurnsAfterCompression);
        if (compressibleCount < 2)
        {
            UpdateStatusBar("warning", "上下文接近上限", "可压缩的历史消息太少，将继续发送当前请求。");
            return;
        }

        UpdateStatusBar("info", "正在压缩上下文", "历史对话已接近上限，正在自动生成摘要...");

        var compressibleTurns = session.Turns
            .Take(compressibleCount)
            .Where(turn => !string.IsNullOrWhiteSpace(turn.Text))
            .Select(turn => new ConversationTurn(turn.Role, turn.Text))
            .ToArray();
        if (compressibleTurns.Length < 2)
        {
            return;
        }

        using var cts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        cts.CancelAfter(TimeSpan.FromSeconds(30));
        var summary = await _backendClient.SendAsync<ConversationSummary>(
            "compressChatHistory",
            new
            {
                summary = session.Summary,
                history = compressibleTurns
            },
            cts.Token);
        if (summary is null)
        {
            return;
        }

        // Re-fetch the latest session inside the lock to avoid discarding
        // messages added by AddTurnAsync during the async compression call (#2267).
        var resolvedId = session.Id;
        await _chatStateLock.WaitAsync();
        try
        {
            var latestSession = _chatState.Sessions.FirstOrDefault(s => s.Id == resolvedId);
            if (latestSession is null)
            {
                return;
            }

            var now = DateTimeOffset.UtcNow.ToString("O");
            var updated = latestSession with
            {
                Summary = summary,
                Turns = latestSession.Turns.Skip(compressibleCount).ToArray(),
                UpdatedAt = now
            };
            var sessions = _chatState.Sessions
                .Select(item => item.Id == updated.Id ? updated : item)
                .ToArray();
            _chatState = new ChatState(updated.Id, sessions);
            _currentSessionId = updated.Id;
            InvalidateTokenEstimate(resolvedId);
        }
        finally
        {
            _chatStateLock.Release();
        }
        await SaveChatStateAsync();
        RefreshSessions();
        ClearRenderCache();
        RenderCurrentSession();
    }

    // ── Turn management ──

    private async Task AddTurnAsync(
        string role,
        string text,
        GroundedAnswer? answer = null,
        IReadOnlyList<ChatAttachment>? attachments = null,
        string? sessionId = null,
        string? source = null)
    {
        await _chatStateLock.WaitAsync();
        try
        {
            ChatSession? session;
            if (sessionId is not null)
            {
                session = _chatState.Sessions.FirstOrDefault(s => s.Id == sessionId);
            }
            else
            {
                session = CurrentSession();
            }
            if (session is null)
            {
                // #3046: When the requested sessionId is stale / invalid,
                // EnsureCurrentSession() either restores an existing session or
                // mints a brand-new one — but its new id will never equal the
                // original (invalid) sessionId, so re-querying by sessionId would
                // still return null and silently drop the turn. Fall back to
                // CurrentSession() (which honours _currentSessionId) instead.
                EnsureCurrentSession();
                session = CurrentSession();
            }

            if (session is null)
            {
                // Defensive: should be unreachable after EnsureCurrentSession(),
                // but log instead of silently returning so a future regression
                // is diagnosable rather than a silent message drop.
                System.Diagnostics.Debug.WriteLine(
                    $"AddTurnAsync: session lookup failed (sessionId={sessionId}, " +
                    $"currentSessionId={_currentSessionId}, sessions={_chatState.Sessions.Count})");
                return;
            }

            var now = DateTimeOffset.UtcNow.ToString("O");
            var turn = new ChatTurn(
                Guid.NewGuid().ToString("N"),
                role,
                text,
                answer?.Citations,
                answer?.SavedNote,
                answer?.ThinkingTrace,
                attachments ?? Array.Empty<ChatAttachment>(),
                now,
                source);

            var turns = new List<ChatTurn>(session.Turns.Count + 1);
            turns.AddRange(session.Turns);
            turns.Add(turn);
            // #2834: bound in-memory history so memory cannot grow without limit
            // in a long-lived session (compression handles summarization; this is
            // a safety net that keeps only the most recent turns).
            if (turns.Count > MaxTurnsPerSession)
            {
                turns.RemoveRange(0, turns.Count - MaxTurnsPerSession);
            }
            var title = session.Title == "新对话" && role == "user"
                ? BuildSessionTitle(text)
                : session.Title;
            var updated = session with { Title = title, Turns = turns, UpdatedAt = now };
            // #3581: O(n) prepend — the updated session is always the most recent.
            var sessions = new List<ChatSession>(_chatState.Sessions.Count);
            sessions.Add(updated);
            foreach (var item in _chatState.Sessions)
            {
                if (item.Id != updated.Id)
                    sessions.Add(item);
            }

            _chatState = new ChatState(updated.Id, sessions.ToArray());
            _currentSessionId = updated.Id;
            InvalidateTokenEstimate(updated.Id);
        }
        finally
        {
            _chatStateLock.Release();
        }
    }

    private async Task SaveChatStateAsync()
    {
        ChatState snapshot;
        await _chatStateLock.WaitAsync();
        try
        {
            snapshot = _chatState;
        }
        finally
        {
            _chatStateLock.Release();
        }

        try
        {
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            await _backendClient.SendAsync<ChatState>(
                "saveChatState",
                new { state = snapshot },
                cts.Token);
        }
        catch (Exception error)
        {
            UpdateStatusBar("warning", "聊天记录未保存", LocalizeError(error.Message));
        }
    }

    private void RefreshSessions()
    {
        // #3581 / #3757: Always rebuild the session list. An early-return
        // optimisation (removed here) compared only session IDs and skipped
        // rebuilds — but after CompressCurrentSessionIfNeededAsync truncates
        // turns, TurnCount / Title / RelativeTime would differ while IDs
        // stayed the same, so stale values were displayed.
        var sessionList = _chatState.Sessions;
        var items = sessionList
            .Select(session => new SessionListItem(
                session.Id,
                session.Title,
                $"{session.Turns.Count} 条消息",
                ToRelativeTime(session.UpdatedAt),
                session.Turns.Count,
                session.Summary?.Text is { Length: > 0 } summary
                    ? (summary.Length <= 50 ? summary : $"{summary[..50]}...")
                    : string.Empty))
            .ToList();

        SessionList.ItemsSource = items;
        SessionList.SelectedItem = SessionList.Items
            .OfType<SessionListItem>()
            .FirstOrDefault(item => item.Id == _currentSessionId);
        DeleteSessionButton.IsEnabled = _chatState.Sessions.Count > 0;
    }

    // ── Session list item ──

    private sealed record SessionListItem(
        string Id,
        string Title,
        string Detail,
        string RelativeTime,
        int TurnCount,
        string SummaryPreview);
}
