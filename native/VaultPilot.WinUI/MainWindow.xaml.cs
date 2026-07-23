using VaultPilot.WinUI.Backend;
using VaultPilot.WinUI.Controls;
using VaultPilot.WinUI.Models;
using VaultPilot.WinUI.Views;
using Microsoft.UI;
using Microsoft.UI.Input;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Linq;
using System.Threading;
using Windows.ApplicationModel.DataTransfer;
using Windows.Foundation;
using Windows.Graphics;
using Windows.Storage;
using Windows.Storage.Pickers;
using Windows.System;
using WinRT.Interop;

namespace VaultPilot.WinUI;

public sealed partial class MainWindow : Window
{
    private const int DefaultWindowWidth = 960;
    private const int DefaultWindowHeight = 760;
    private const int MinimumWindowWidth = 640;
    private const int MinimumWindowHeight = 520;
    private const double AutoCollapseSidebarWidth = 1040;
    private const int WindowMessageDropFiles = 0x0233;
    private const int WindowLongPtrWndProc = -4;
    private static readonly string ClipboardAttachmentDirectory = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "com.local.vaultpilot",
        "clipboard-images");
    private const int MaxClipboardImages = 50;
    private readonly BackendClient _backendClient;
    private AppWindow? _appWindow;
    // Theme-aware status brushes — looked up from ThemeResource each call so they
    // automatically track theme changes (light/dark).  The dictionary lookup is O(1).
    private static Brush BrushRed => GetThemeBrush("StatusErrorBrush");
    private static Brush BrushOrange => GetThemeBrush("StatusWarningBrush");
    private static Brush BrushGreen => GetThemeBrush("StatusSuccessBrush");
    private static Brush BrushLimeGreen => GetThemeBrush("SystemFillColorSuccessBrush");

    private AppSettings? _settings;
    private int _noteCount;
    private Dictionary<string, string>? _noteTitleMap;
    private DateTime _noteTitleMapTimestamp;
    private const int NoteTitleMapTtlMs = 30_000;
    private bool _sidebarCollapsed = true;
    private bool _sidebarAutoCollapsed = true;
    private string _startupStep = "初始化";
    private volatile int _updateDownloadPercent = -1;
    private volatile string _updateDownloadVersion = string.Empty;
    private DispatcherTimer? _autoWakeTimer;
    private int _autoWakeConsecutiveFailures;
    private const int AutoWakeMaxFailures = 3;
    private Views.NotesView? _notesView;
    private bool _notesViewLoaded;
    private nint _windowHandle;
    private nint _originalWindowProc;
    private WindowProcDelegate? _windowProcDelegate;
    private GCHandle _windowProcDelegateHandle;

    // ── Note navigation history (#3230) ────────────────────
    private readonly List<string> _noteNavStack = new();
    private int _noteNavIndex = -1;

    public MainWindow()
    {
        InitializeComponent();
        ConfigureWindowBounds();
        _backendClient = new BackendClient();
        _backendClient.AgentStatusReceived += OnAgentStatusReceived;
        _backendClient.ConnectionStateChanged += OnConnectionStateChanged;
        RootGrid.Loaded += OnLoaded;
        Closed += OnClosed;
        SendButton.Click += OnSendClicked;
        RecordButton.Click += OnRecordClicked;
        CancelButton.Click += (_, _) => CancelActiveRequest();
        SettingsButton.Click += OnSettingsClicked;
        RebuildButton.Click += OnRebuildClicked;
        ImportButton.Click += OnImportClicked;
        ComposerBox.KeyDown += OnComposerKeyDown;
        SessionList.SelectionChanged += OnSessionSelectionChanged;
        DeleteSessionButton.Click += OnDeleteSessionClicked;
        NewSessionButton.Click += OnNewSessionClicked;
        ToggleSidebarButton.Click += OnToggleSidebarClicked;
        ExpandSidebarButton.Click += OnExpandSidebarClicked;
        ChatScrollViewer.ViewChanged += OnChatScrollViewerViewChanged;
        JumpLatestButton.Click += OnJumpLatestClicked;
        RootGrid.SizeChanged += OnRootGridSizeChanged;

        // KeyboardAccelerators for keys that the WinUI XamlCompiler cannot parse (OemComma, Number1, Number2)
        AddKeyboardAccelerator((VirtualKey)188 /* OemComma */, VirtualKeyModifiers.Control, OnSettingsAccelerator);
        AddKeyboardAccelerator(VirtualKey.Number1, VirtualKeyModifiers.Control, OnNavChatAccelerator);
        AddKeyboardAccelerator(VirtualKey.Number2, VirtualKeyModifiers.Control, OnNavNotesAccelerator);

        // Note navigation history (#3230)
        AddKeyboardAccelerator(VirtualKey.Left, VirtualKeyModifiers.Menu, OnNavigateBack);
        AddKeyboardAccelerator(VirtualKey.Right, VirtualKeyModifiers.Menu, OnNavigateForward);

        // Initialize AI command palette (#2188)
        AiCommandPaletteControl.Backend = _backendClient;
        AiCommandPaletteControl.InsertToChatRequested += OnPaletteInsertToChat;

        // Initialize Quick Ask overlay (#1799)
        QuickAskControl.Backend = _backendClient;
        QuickAskControl.InsertToNoteRequested += OnQuickAskInsertToNote;
    }

    private void AddKeyboardAccelerator(VirtualKey key, VirtualKeyModifiers modifiers, TypedEventHandler<KeyboardAccelerator, KeyboardAcceleratorInvokedEventArgs> handler)
    {
        var accel = new KeyboardAccelerator { Key = key, Modifiers = modifiers };
        accel.Invoked += handler;
        RootGrid.KeyboardAccelerators.Add(accel);
    }

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        try
        {
            // Apply persisted theme as early as possible to avoid a flash of the
            // wrong theme during backend startup.
            ApplyTheme();
            await LogStartup("Window loaded");
            await UpdateStartupStepAsync("启动后端");
            var backendPath = ResolveBackendPath();
            await LogStartup($"Backend path: {backendPath}");
            await _backendClient.StartAsync(backendPath);
            await LogStartup("Backend process started");
            await UpdateStartupStepAsync("检查后端响应");
            await SendWithTimeoutAsync(
                (token) => _backendClient.SendAsync("ping", new { }, token),
                "ping");
            await LogStartup("Ping ok");

            await UpdateStartupStepAsync("读取设置");
            _settings = await SendWithTimeoutAsync(
                (token) => _backendClient.SendAsync<AppSettings>("getSettings", new { }, token),
                "getSettings");

            await UpdateStartupStepAsync("读取聊天记录");
            _chatState = await TryLoadChatStateAsync();

            await UpdateStartupStepAsync("读取笔记列表");
            _noteCount = await TryLoadNoteCountAsync();
            EnsureCurrentSession();

            RefreshVaultSummary();
            RefreshSessions();
            SetSidebarCollapsed(collapsed: true, autoCollapsed: true);
            RenderCurrentSession();
            ScrollToLatest();

            UpdateStatusBar("success", "后端已连接", "就绪");
            await LogStartup("Startup complete");
            ApplyAutoWakeSettings();
            ShowNextWakeTime();
            if (_settings?.AutoCheckUpdates ?? true)
            {
                _ = CheckForAppUpdatesAsync();
            }
            else
            {
                await LogStartup("Update check skipped: disabled in settings.");
            }
        }
        catch (Exception error)
        {
            await ShowStartupFailureAsync(error, _backendClient.GetStderrTail());
            await _backendClient.DisposeAsync();
            Close();
        }
    }

    private void ConfigureWindowBounds()
    {
        try
        {
            var hwnd = WindowNative.GetWindowHandle(this);
            _windowHandle = hwnd;
            EnsureWindowFileDropHook(hwnd);
            var windowId = Win32Interop.GetWindowIdFromWindow(hwnd);
            _appWindow = AppWindow.GetFromWindowId(windowId);
            _appWindow.Resize(new SizeInt32(DefaultWindowWidth, DefaultWindowHeight));

            var iconPath = Path.Combine(AppContext.BaseDirectory, "icon.ico");
            if (File.Exists(iconPath))
            {
                _appWindow.SetIcon(iconPath);
            }
        }
        catch
        {
            _appWindow = null;
        }
    }

    /// <summary>
    /// Applies the persisted theme preference to the root element. Called once
    /// on startup (OnLoaded) and again after the user changes it in Settings.
    /// <see cref="ElementTheme.Default"/> follows the OS theme.
    /// </summary>
    private void ApplyTheme(ElementTheme? mode = null)
    {
        var theme = mode ?? ThemePreferences.Load();
        if (RootGrid.RequestedTheme != theme)
        {
            RootGrid.RequestedTheme = theme;
        }
    }

    private async Task<int> TryLoadNoteCountAsync()
    {
        try
        {
            var notes = await SendWithTimeoutAsync(
                (token) => _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { }, token),
                "listNotes");
            return notes?.Count ?? 0;
        }
        catch (Exception error)
        {
            AppendMessage("错误", $"笔记列表读取失败：{LocalizeError(error.Message)}");
            return 0;
        }
    }

    private async void OnSettingsClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            try
            {
                using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
                _settings ??= await _backendClient.SendAsync<AppSettings>("getSettings", new { }, cts.Token);
            }
            catch (Exception loadError)
            {
                ShowError("无法加载设置", new InvalidOperationException($"设置加载失败，请检查后端连接：{loadError.Message}", loadError));
                return;
            }
            if (_settings is null)
            {
                ShowError("无法加载设置", new InvalidOperationException("后端返回了空设置数据，请重启应用后重试。"));
                return;
            }

            // Compute next wake time text for display in the dialog.
            string? nextWakeText = null;
            if (_settings.AutoWakeEnabled)
            {
                var next = GetNextAutoWakeTime();
                if (next.HasValue)
                {
                    nextWakeText = next.Value.Date == DateTime.Today
                        ? $"下次唤醒: {next.Value:HH:mm}"
                        : $"下次唤醒: {next.Value:MM/dd HH:mm}";
                }
            }

            // Use the active provider from the multi-provider list, with
            // fallback to the legacy single Provider field for backward compat.
            // Defensive null-coalescing (issue #3090): System.Text.Json can
            // explicitly set Provider to null when the backend payload contains
            // "provider": null, which would otherwise NullReferenceException
            // on the .BaseUrl access below.
            var activeProvider = _settings.Providers.Count > 0
                ? _settings.Providers[Math.Clamp(_settings.ActiveProviderIndex, 0, _settings.Providers.Count - 1)]
                : (_settings.Provider ?? new ProviderConfig());
            var models = GetModelsForProvider(activeProvider?.BaseUrl ?? string.Empty);

            var dialog = new Views.SettingsDialog(
                _settings,
                models,
                nextWakeText,
                ResolveDisplayVersion(),
                RootGrid.XamlRoot,
                OpenVaultDirectoryAsync,
                OpenProjectHomepageAsync);

            await dialog.ShowAsync();

            if (dialog.UpdatedSettings is { } updated)
            {
                using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
                _settings = await _backendClient.SendAsync<AppSettings>("saveSettings", new { settings = updated }, cts.Token);
                RefreshVaultSummary();
                RefreshContextStatus();
                ApplyAutoWakeSettings();
                UpdateStatusBar("success", "设置已保存", "模型服务配置已更新。");
                ShowNextWakeTime();
                // Apply theme change immediately so the user sees it without restart.
                ApplyTheme(dialog.ThemeMode);
            }
        }
        catch (Exception error)
        {
            ShowError("打开设置失败", error);
        }
    }

    private async void OnRebuildClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            RebuildButton.IsEnabled = false;
            UpdateStatusBar("info", "正在重建索引", "正在扫描知识库...");

            using var rebuildCts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            var stats = await _backendClient.SendAsync<IndexStats>("rebuildIndex", new { }, rebuildCts.Token);
            using var listCts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            var notes = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { }, listCts.Token);
            _noteCount = notes?.Count ?? 0;
            RefreshVaultSummary();
            InvalidateNoteTitleCache();

            UpdateStatusBar("success", "索引已重建", $"扫描 {stats?.Scanned ?? 0}，索引 {stats?.Indexed ?? 0}，移除 {stats?.Removed ?? 0}。");
        }
        catch (Exception error)
        {
            ShowError("重建索引失败", error);
        }
        finally
        {
            RebuildButton.IsEnabled = true;
        }
    }

    private async void OnImportClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            var picker = new FileOpenPicker
            {
                SuggestedStartLocation = PickerLocationId.DocumentsLibrary
            };
            picker.FileTypeFilter.Add(".md");
            picker.FileTypeFilter.Add(".markdown");
            InitializeWithWindow.Initialize(picker, WindowNative.GetWindowHandle(this));

            var files = await picker.PickMultipleFilesAsync();
            if (files.Count == 0)
            {
                return;
            }

            ImportButton.IsEnabled = false;
            UpdateStatusBar("info", "正在导入", $"正在导入 {files.Count} 个 Markdown 文件...");

            var paths = files.Select(file => file.Path).ToArray();
            using var importCts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            var result = await _backendClient.SendAsync<ImportResult>("importMarkdown", new { paths }, importCts.Token);
            using var listCts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            var notes = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { }, listCts.Token);
            _noteCount = notes?.Count ?? 0;
            RefreshVaultSummary();
            InvalidateNoteTitleCache();

            UpdateStatusBar(result?.Errors.Count > 0 ? "warning" : "success", "导入完成", $"导入 {result?.Imported ?? 0}，跳过 {result?.Skipped ?? 0}，错误 {result?.Errors.Count ?? 0}。");
        }
        catch (Exception error)
        {
            ShowError("导入失败", error);
        }
        finally
        {
            ImportButton.IsEnabled = true;
        }
    }

    private void OnToggleSidebarClicked(object sender, RoutedEventArgs e)
    {
        SetSidebarCollapsed(collapsed: true, autoCollapsed: false);
    }

    private void OnExpandSidebarClicked(object sender, RoutedEventArgs e)
    {
        SetSidebarCollapsed(collapsed: false, autoCollapsed: false);
    }

    private void OnActivityChatClicked(object sender, RoutedEventArgs e)
    {
        SwitchToChatView();
    }

    private void OnActivityNotesClicked(object sender, RoutedEventArgs e)
    {
        _ = SwitchToNotesViewAsync();
    }

    /// <summary>Shows the chat view, hides notes.</summary>
    private void SwitchToChatView()
    {
        ChatView.Visibility = Visibility.Visible;
        NotesViewHost.Visibility = Visibility.Collapsed;
    }

    /// <summary>Shows the notes view, lazily initializing it on first use.</summary>
    private async Task SwitchToNotesViewAsync()
    {
        try
        {
            ChatView.Visibility = Visibility.Collapsed;
            NotesViewHost.Visibility = Visibility.Visible;
            if (!_notesViewLoaded)
            {
                _notesView = new Views.NotesView(_backendClient);
                NotesViewHost.Children.Add(_notesView);
                _notesViewLoaded = true;
            }
            await _notesView.RefreshNotesAsync();
        }
        catch (Exception error)
        {
            Debug.WriteLine($"[SwitchToNotesViewAsync] Error: {error}");
        }
    }

    private void OnRootGridSizeChanged(object sender, SizeChangedEventArgs e)
    {
        EnforceMinimumWindowSize();

        if (e.NewSize.Width < AutoCollapseSidebarWidth && !_sidebarCollapsed)
        {
            SetSidebarCollapsed(collapsed: true, autoCollapsed: true);
        }
        else if (e.NewSize.Width >= AutoCollapseSidebarWidth && _sidebarAutoCollapsed)
        {
            SetSidebarCollapsed(collapsed: false, autoCollapsed: false);
        }
    }

    private void SetSidebarCollapsed(bool collapsed, bool autoCollapsed)
    {
        _sidebarCollapsed = collapsed;
        _sidebarAutoCollapsed = collapsed && autoCollapsed;
        SidebarBorder.Visibility = collapsed ? Visibility.Collapsed : Visibility.Visible;
        SidebarColumn.Width = collapsed ? new GridLength(0) : new GridLength(260);
        // Toggle which sidebar affordance is visible:
        // collapsed → show "expand" button in the top bar; expanded → show "collapse" inside the panel.
        ExpandSidebarButton.Visibility = collapsed ? Visibility.Visible : Visibility.Collapsed;
    }

    private void EnforceMinimumWindowSize()
    {
        if (_appWindow is null)
        {
            return;
        }

        var width = Math.Max(_appWindow.Size.Width, MinimumWindowWidth);
        var height = Math.Max(_appWindow.Size.Height, MinimumWindowHeight);
        if (width != _appWindow.Size.Width || height != _appWindow.Size.Height)
        {
            _appWindow.Resize(new SizeInt32(width, height));
        }
    }

    private Task OpenVaultDirectoryAsync()
    {
        var vaultDir = _settings?.VaultDir?.Trim();
        if (string.IsNullOrWhiteSpace(vaultDir))
        {
            ShowError("打开目录失败", new InvalidOperationException("尚未配置知识库目录。"), addMessage: false);
            return Task.CompletedTask;
        }

        if (!Directory.Exists(vaultDir))
        {
            ShowError("打开目录失败", new DirectoryNotFoundException("知识库目录不存在。"), addMessage: false);
            return Task.CompletedTask;
        }

        try
        {
            using var launched = Process.Start(new ProcessStartInfo
            {
                FileName = vaultDir,
                UseShellExecute = true,
                Verb = "open"
            });
            if (launched is null)
            {
                throw new InvalidOperationException("系统未能打开知识库目录。");
            }
        }
        catch (Exception error)
        {
            ShowError("打开目录失败", error);
        }

        return Task.CompletedTask;
    }

    private async Task OpenProjectHomepageAsync()
    {
        try
        {
            var launched = await Launcher.LaunchUriAsync(new Uri(UpdateRepoUrl));
            if (!launched)
            {
                throw new InvalidOperationException("系统未能打开项目地址。");
            }
        }
        catch (Exception error)
        {
            ShowError("打开项目地址失败", error, addMessage: false);
        }
    }

    private void OnComposerDropZoneDragOver(object sender, DragEventArgs e)
    {
        if (!e.DataView.Contains(StandardDataFormats.StorageItems))
        {
            e.AcceptedOperation = DataPackageOperation.None;
            return;
        }

        e.AcceptedOperation = DataPackageOperation.Copy;
    }

    private async void OnComposerDropZoneDrop(object sender, DragEventArgs e)
    {
        try
        {
            if (!e.DataView.Contains(StandardDataFormats.StorageItems))
            {
                return;
            }

            var items = await e.DataView.GetStorageItemsAsync();
            var files = items
                .OfType<StorageFile>()
                .Where(IsSupportedImageFile)
                .ToArray();

            if (files.Length == 0)
            {
                UpdateStatusBar("warning", "未添加图片", "拖入的文件里没有可用的图片。");
                return;
            }

            AddImageAttachments(files);
        }
        catch (Exception error)
        {
            ShowError("拖放图片失败", error);
        }
    }

    private volatile bool _isShuttingDown;

    private void OnClosed(object sender, WindowEventArgs args)
    {
        if (_isShuttingDown)
        {
            // ShutdownAsync already performed full cleanup; nothing to do.
            return;
        }

        // We are NOT truly exiting — this is a hide-to-tray close.
        // Do NOT cancel the active request — let it complete in the background
        // so the user doesn't lose their in-flight AI response (issue #636).
        // Do NOT dispose the backend or unsubscribe events so the window
        // can be re-shown from the tray.
        try
        {
            RemoveThinkingIndicator();
        }
        catch (Exception error)
        {
            ShowError("关闭窗口失败", error);
        }
    }

    /// <summary>
    /// Unsubscribes all event handlers registered in the constructor to prevent
    /// memory leaks from dangling references after the window is closed.
    /// </summary>
    private void UnsubscribeEvents()
    {
        _backendClient.AgentStatusReceived -= OnAgentStatusReceived;
        _backendClient.ConnectionStateChanged -= OnConnectionStateChanged;
        RootGrid.Loaded -= OnLoaded;
        SendButton.Click -= OnSendClicked;
        RecordButton.Click -= OnRecordClicked;
        SettingsButton.Click -= OnSettingsClicked;
        RebuildButton.Click -= OnRebuildClicked;
        ImportButton.Click -= OnImportClicked;
        ComposerBox.KeyDown -= OnComposerKeyDown;
        ComposerBox.TextChanged -= OnComposerTextChanged;
        SessionList.SelectionChanged -= OnSessionSelectionChanged;
        DeleteSessionButton.Click -= OnDeleteSessionClicked;
        NewSessionButton.Click -= OnNewSessionClicked;
        ToggleSidebarButton.Click -= OnToggleSidebarClicked;
        ChatScrollViewer.ViewChanged -= OnChatScrollViewerViewChanged;
        JumpLatestButton.Click -= OnJumpLatestClicked;
        RootGrid.SizeChanged -= OnRootGridSizeChanged;
        AiCommandPaletteControl.InsertToChatRequested -= OnPaletteInsertToChat;
        QuickAskControl.InsertToNoteRequested -= OnQuickAskInsertToNote;
    }

    #region Keyboard Accelerator Handlers

    private void OnNewSessionAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        OnNewSessionClicked(NewSessionButton, new RoutedEventArgs());
    }

    private void OnToggleSidebarAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        OnToggleSidebarClicked(ToggleSidebarButton, new RoutedEventArgs());
    }

    private void OnSettingsAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        OnSettingsClicked(SettingsButton, new RoutedEventArgs());
    }

    private void OnEscapeAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        // If the Quick Ask overlay is open, dismiss it first
        if (QuickAskControl.Visibility == Visibility.Visible)
        {
            QuickAskControl.Dismiss();
            return;
        }
        // If the AI command palette is open, dismiss it next
        if (AiCommandPaletteControl.Visibility == Visibility.Visible)
        {
            AiCommandPaletteControl.Dismiss();
            return;
        }
        CancelActiveRequest();
    }

    private void OnAiCommandPaletteAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        ShowAiCommandPalette();
    }

    private void OnQuickAskAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        ShowQuickAsk();
    }

    private void OnNavChatAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        SwitchToChatView();
    }

    private void OnNavNotesAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        _ = SwitchToNotesViewAsync();
    }

    #endregion

    public void Hide()
    {
        _appWindow?.Hide();
    }

    public void ShowAndActivate()
    {
        if (_appWindow != null)
        {
            _appWindow.Show();
        }

        Activate();
    }

    public async Task PrepareExitAsync()
    {
        await ShutdownAsync();
    }

    /// <summary>
    /// Performs all cleanup (backend client disposal, resource release) before
    /// the application exits.  Called from the tray "Exit" handler so that
    /// MainWindow.OnClosed logic is executed even when the window is only
    /// hidden to tray.  See: https://github.com/user/repo/issues/62
    /// </summary>
    public async Task ShutdownAsync()
    {
        _isShuttingDown = true;

        // Cancel any active AI request before releasing resources
        // to prevent catch/finally blocks from accessing disposed objects.
        // #3097: Only Cancel, don't dispose or null out _activeRequestCts.
        // ExecuteAiRequestAsync's finally block is the sole owner of disposal (#2732).
        var activeCts = Volatile.Read(ref _activeRequestCts);
        try { activeCts?.Cancel(); } catch (ObjectDisposedException) { }

        // Wait for the active AI request to finish its catch/finally cleanup
        // before disposing shared resources (#446)
        var activeTask = Volatile.Read(ref _activeRequestTask);
        if (activeTask != null)
        {
            try
            {
                await activeTask.WaitAsync(TimeSpan.FromSeconds(35));
            }
            catch (TimeoutException)
            {
                // Proceed with disposal even if the request doesn't finish in time
            }
        }

        // Cancel Agent mode to prevent ObjectDisposedException from
        // ExecuteAgentRequestAsync accessing disposed _backendClient (#2304)
        // #2822: Only Cancel, don't Dispose — consistent with CancelActiveRequest pattern (#2732).
        // Dispose is the sole responsibility of the CTS owner's finally block.
        var agentCts = Interlocked.Exchange(ref _agentCts, null);
        agentCts?.Cancel();

        RemoveThinkingIndicator();
        StopAutoWakeTimer();
        UnsubscribeEvents();
        TryReleaseWindowFileDropHook();
        await SaveChatStateAsync();
        _chatStateLock?.Dispose();
        await _backendClient.DisposeAsync();
        PruneClipboardImages();
    }

    private void OnAgentStatusReceived(AgentStatusEvent status)
    {
        DispatcherQueue.TryEnqueue(() =>
        {
            // Route agent-mode events to HandleAgentEvent
            if (_agentModeActive)
            {
                HandleAgentEvent(status);
            }
            else
            {
                UpdateStatusBar("info", LocalizeStage(status.Stage), LocalizeStatusDetail(status.Detail));
            }
        });
    }

    private void OnConnectionStateChanged(bool connected)
    {
        DispatcherQueue.TryEnqueue(() =>
        {
            if (connected)
            {
                UpdateStatusBar("success", "后端已连接", "连接已恢复");
            }
            else
            {
                UpdateStatusBar("warning", "后端断开", "正在尝试重新连接...");
            }
        });
    }

    private static string ResolveBackendPath()
    {
        var outputPath = Path.Combine(AppContext.BaseDirectory, "vaultpilot-agent.exe");
        if (File.Exists(outputPath))
        {
            return outputPath;
        }

        var repoPath = Path.GetFullPath(Path.Combine(
            AppContext.BaseDirectory,
            "..",
            "..",
            "..",
            "..",
            "..",
            "target",
            "debug",
            "vaultpilot-agent.exe"));
        if (File.Exists(repoPath))
        {
            return repoPath;
        }

        throw new FileNotFoundException("未找到 vaultpilot-agent.exe。请先构建 Rust sidecar。", outputPath);
    }

    private void EnsureWindowFileDropHook(nint hwnd)
    {
        if (hwnd == 0 || _windowProcDelegate is not null)
        {
            return;
        }

        _windowProcDelegate = WindowProc;
        _windowProcDelegateHandle = GCHandle.Alloc(_windowProcDelegate);
        var newWindowProc = Marshal.GetFunctionPointerForDelegate(_windowProcDelegate);
        _originalWindowProc = SetWindowLongPtr(hwnd, WindowLongPtrWndProc, newWindowProc);
        DragAcceptFiles(hwnd, true);
    }

    #region Auto-wake timer

    private DateTime? _lastAutoWakeTime;
    private volatile int _autoWakeInProgress;
    private volatile bool _isStopping; // #677: volatile for cross-thread visibility

    /// <summary>
    /// Called by App during shutdown to prevent the auto-wake timer
    /// from firing while the app is exiting.
    /// </summary>
    public void SignalStopping()
    {
        _isStopping = true;
        StopAutoWakeTimer();
    }

    private static bool IsAnthropicProvider(string? baseUrl)
    {
        if (string.IsNullOrEmpty(baseUrl)) return false;
        return baseUrl.Contains("anthropic", StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>
    /// Returns a curated list of recommended models for the configured provider,
    /// determined by inspecting the base URL. Mirrors the Rust-side
    /// <c>ProviderType::from_base_url</c> heuristic.
    /// </summary>
    private static string[] GetModelsForProvider(string? baseUrl)
    {
        if (IsAnthropicProvider(baseUrl))
        {
            return new[]
            {
                "claude-3-5-haiku-latest",
                "claude-3-5-sonnet-latest",
                "claude-sonnet-4-20250514",
            };
        }

        // Default: OpenAI-compatible provider (covers OpenAI, Azure, local
        // servers, and third-party gateways that expose the OpenAI API).
        return new[]
        {
            "deepseek-v4-flash-free",
            "mimo-v2.5-free",
            "qwen3.6-plus-free",
            "minimax-m3-free",
            "big-pickle",
            "google/gemma-4-31b-it:free",
            "gpt-4o-mini",
            "gpt-4o",
            "gpt-4.1-mini",
            "gpt-4.1",
        };
    }

    private void ApplyAutoWakeSettings()
    {
        StopAutoWakeTimer();

        if (_settings is null || !_settings.AutoWakeEnabled)
        {
            return;
        }

        _lastAutoWakeTime = null;
        _autoWakeConsecutiveFailures = 0;
        _autoWakeTimer = new DispatcherTimer
        {
            Interval = TimeSpan.FromMinutes(1),
        };
        _autoWakeTimer.Tick += OnAutoWakeTimerTick;
        _autoWakeTimer.Start();
    }

    private void StopAutoWakeTimer()
    {
        if (_autoWakeTimer != null)
        {
            _autoWakeTimer.Stop();
            _autoWakeTimer.Tick -= OnAutoWakeTimerTick;
            _autoWakeTimer = null;
        }
        _lastAutoWakeTime = null;
    }

    private bool IsInAutoWakeWindow()
    {
        var settings = _settings;
        if (settings == null) return false;

        if (string.IsNullOrEmpty(settings.AutoWakeStartTime) && string.IsNullOrEmpty(settings.AutoWakeEndTime))
        {
            return true;
        }

        var now = DateTime.Now.TimeOfDay;
        if (!TimeSpan.TryParse(settings.AutoWakeStartTime, out var startTime)) startTime = TimeSpan.Zero;
        if (!TimeSpan.TryParse(settings.AutoWakeEndTime, out var endTime)) endTime = TimeSpan.FromHours(24);

        if (startTime <= endTime)
        {
            return now >= startTime && now <= endTime;
        }
        // Cross-midnight: e.g. 22:00 to 06:00
        return now >= startTime || now <= endTime;
    }

    private async void OnAutoWakeTimerTick(object? sender, object e)
    {
        if (_isStopping) return;
        if (_autoWakeInProgress != 0) return;
        if (!IsInAutoWakeWindow()) return;

        var interval = TimeSpan.FromMinutes((int)Math.Clamp((long)(_settings?.AutoWakeIntervalMinutes ?? 30), 1, 1440));
        var now = DateTime.Now;
        if (_lastAutoWakeTime.HasValue && (now - _lastAutoWakeTime.Value) < interval) return;

        if (Interlocked.CompareExchange(ref _autoWakeInProgress, 1, 0) != 0) return;
        try
        {
            await _backendClient.EnsureConnectedAsync();

            // #861: Send an actual AI prompt and display both the prompt and
            // response in the chat dialog instead of a silent "ping".
            var wakePrompt = _settings?.AutoWakePrompt?.Trim();
            if (string.IsNullOrEmpty(wakePrompt))
            {
                wakePrompt = "请简要回顾一下最近的知识库内容，给我一个简短的今日摘要。";
            }

            // Add the wake prompt as a user message with ⏰ marker
            var requestSessionId = _currentSessionId;
            await AddTurnAsync("user", $"⏰ {wakePrompt}", sessionId: requestSessionId, source: "scheduled_wake");
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();

            // Send to AI
            var history = GetConversationHistory(requestSessionId);
            // Defensive: _settings?.Provider only guards _settings, not Provider
            // itself — System.Text.Json can leave Provider null if the backend
            // explicitly sent "provider": null (issue #3090).
            var timeout = ResolveActiveProvider().RequestTimeoutMs;
            using var cts = new CancellationTokenSource(
                TimeSpan.FromMilliseconds((timeout > 0 ? timeout : 60_000) + 30_000));
            var wakeModelOverride = _settings?.AutoWakeModel?.Trim();
            var answer = await _backendClient.SendAsync<GroundedAnswer>(
                "askWithAi",
                new
                {
                    question = wakePrompt,
                    history,
                    imagePaths = Array.Empty<string>(),
                    modelOverride = string.IsNullOrEmpty(wakeModelOverride) ? null : wakeModelOverride
                },
                cts.Token);

            // Add the AI response as an assistant message
            await AddTurnAsync("assistant", answer?.Answer ?? "(无回复)", answer, sessionId: requestSessionId, source: "scheduled_wake");
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();

            _lastAutoWakeTime = DateTime.Now;
            _autoWakeConsecutiveFailures = 0; // reset counter on success
            await LogStartup("自动唤醒完成: 已发送提问并收到回复");
        }
        catch (Exception error)
        {
            // Back off: update last wake time so the interval is respected
            _lastAutoWakeTime = DateTime.Now;
            _autoWakeConsecutiveFailures++;

            if (_autoWakeConsecutiveFailures >= AutoWakeMaxFailures)
            {
                // After 3 consecutive failures, disable auto-wake to prevent
                // flooding the UI with retries.
                _autoWakeConsecutiveFailures = 0;
                var msg = $"⏰ 自动唤醒失败 {AutoWakeMaxFailures} 次，已暂停自动唤醒。请修复后重启或重新启用。";
                var failSessionId = _currentSessionId;
                await AddTurnAsync("assistant", msg, sessionId: failSessionId, source: "scheduled_wake");
                RenderCurrentSession();
                ScrollToLatest();
                if (!_isShuttingDown)
                {
                    await SaveChatStateAsync();
                }
                await LogStartup(msg);
                StopAutoWakeTimer();
            }
            else
            {
                // Add error as assistant message so user can see what happened
                var errorSessionId = _currentSessionId;
                await AddTurnAsync("assistant", $"⏰ 自动唤醒失败: {LocalizeError(error.Message)}", sessionId: errorSessionId, source: "scheduled_wake");
                RenderCurrentSession();
                ScrollToLatest();
                if (!_isShuttingDown)
                {
                    await SaveChatStateAsync();
                }
                await LogStartup($"自动唤醒失败: {LocalizeError(error.Message)}");
            }
        }
        finally
        {
            Interlocked.Exchange(ref _autoWakeInProgress, 0);
            ShowNextWakeTime();
        }
    }

    private DateTime? GetNextAutoWakeTime()
    {
        var settings = _settings;
        if (settings == null || !settings.AutoWakeEnabled) return null;

        var intervalMinutes = (int)Math.Clamp((long)settings.AutoWakeIntervalMinutes, 1, 1440);
        var interval = TimeSpan.FromMinutes(intervalMinutes);
        var now = DateTime.Now;

        // No time window: simple interval
        if (string.IsNullOrEmpty(settings.AutoWakeStartTime) && string.IsNullOrEmpty(settings.AutoWakeEndTime))
        {
            return _lastAutoWakeTime.HasValue
                ? _lastAutoWakeTime.Value + interval
                : now + interval;
        }

        if (!TimeSpan.TryParse(settings.AutoWakeStartTime, out var startTime)) startTime = TimeSpan.Zero;
        if (!TimeSpan.TryParse(settings.AutoWakeEndTime, out var endTime)) endTime = TimeSpan.FromHours(24);

        // If last wake is known, next is last + interval (if still in window)
        if (_lastAutoWakeTime.HasValue)
        {
            var candidate = _lastAutoWakeTime.Value + interval;
            if (IsTimeInWindow(candidate.TimeOfDay, startTime, endTime))
                return candidate;
            // Past today's window, restart from start_time tomorrow
            return now.Date.AddDays(1) + startTime;
        }

        // No last wake yet: find next slot anchored to start_time
        // Schedule for today: start, start+interval, start+2*interval, ...
        for (var day = 0; day <= 1; day++)
        {
            var baseTime = now.Date.AddDays(day) + startTime;
            // Calculate max slots from window duration to cover the entire window
            var windowDuration = endTime >= startTime
                ? endTime - startTime
                : (TimeSpan.FromHours(24) - startTime) + endTime;
            var maxSlots = (int)(windowDuration.TotalMinutes / intervalMinutes) + 1;
            for (int i = 0; i <= maxSlots; i++)
            {
                var slot = baseTime + TimeSpan.FromTicks(interval.Ticks * i);
                var slotTime = slot.TimeOfDay;

                // Check if still in window
                if (!IsTimeInWindow(slotTime, startTime, endTime)) break;
                if (slot > now) return slot;
            }
        }

        // Fallback: start_time tomorrow
        return now.Date.AddDays(1) + startTime;
    }

    private static bool IsTimeInWindow(TimeSpan time, TimeSpan start, TimeSpan end)
    {
        if (start <= end)
            return time >= start && time <= end;
        return time >= start || time <= end;
    }

    private void ShowNextWakeTime()
    {
        var next = GetNextAutoWakeTime();
        if (next.HasValue)
        {
            var label = next.Value.Date == DateTime.Today
                ? $"下次唤醒: {next.Value:HH:mm}"
                : $"下次唤醒: {next.Value:MM/dd HH:mm}";
            UpdateStatusBar("info", "自动唤醒已启用", label);
        }
    }

    #endregion

    private void TryReleaseWindowFileDropHook()
    {
        if (_windowHandle == 0)
        {
            return;
        }

        try
        {
            DragAcceptFiles(_windowHandle, false);
            if (_originalWindowProc != 0)
            {
                SetWindowLongPtr(_windowHandle, WindowLongPtrWndProc, _originalWindowProc);
            }
        }
        catch
        {
            // Ignore teardown failures.
        }
        finally
        {
            _windowHandle = 0;
            _originalWindowProc = 0;
            if (_windowProcDelegateHandle.IsAllocated)
                _windowProcDelegateHandle.Free();
            _windowProcDelegate = null;
        }
    }

    private nint WindowProc(nint hwnd, uint msg, nint wParam, nint lParam)
    {
        if (msg == WindowMessageDropFiles)
        {
            var paths = ReadDroppedPaths(wParam);
            DragFinish(wParam);
            if (paths.Count > 0)
            {
                DispatcherQueue.TryEnqueue(async () => await HandleWindowFileDropAsync(paths));
            }

            return 0;
        }

        return CallWindowProc(_originalWindowProc, hwnd, msg, wParam, lParam);
    }

    private async Task HandleWindowFileDropAsync(IReadOnlyList<string> paths)
    {
        try
        {
            var imagePaths = paths
                .Where(IsSupportedImagePath)
                .Distinct(StringComparer.OrdinalIgnoreCase)
                .ToArray();

            if (imagePaths.Length == 0)
            {
                UpdateStatusBar("warning", "未添加图片", "拖入的文件里没有可用的图片。");
                return;
            }

            var files = new List<StorageFile>(imagePaths.Length);
            foreach (var path in imagePaths)
            {
                files.Add(await StorageFile.GetFileFromPathAsync(path));
            }

            AddImageAttachments(files);
        }
        catch (Exception error)
        {
            ShowError("拖放图片失败", error);
        }
    }

    private static IReadOnlyList<string> ReadDroppedPaths(nint dropHandle)
    {
        var count = DragQueryFile(dropHandle, 0xFFFFFFFF, null, 0);
        if (count == 0)
        {
            return Array.Empty<string>();
        }

        var result = new List<string>((int)count);
        for (uint index = 0; index < count; index++)
        {
            var length = DragQueryFile(dropHandle, index, null, 0);
            if (length == 0)
            {
                continue;
            }

            var buffer = new StringBuilder((int)length + 1);
            _ = DragQueryFile(dropHandle, index, buffer, (uint)buffer.Capacity);
            result.Add(buffer.ToString());
        }

        return result;
    }

    private delegate nint WindowProcDelegate(nint hwnd, uint msg, nint wParam, nint lParam);

    [DllImport("shell32.dll")]
    private static extern void DragAcceptFiles(nint hwnd, bool accept);

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern uint DragQueryFile(nint hDrop, uint iFile, StringBuilder? lpszFile, uint cch);

    [DllImport("shell32.dll")]
    private static extern void DragFinish(nint hDrop);

    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")]
    private static extern nint SetWindowLongPtr(nint hWnd, int nIndex, nint dwNewLong);

    [DllImport("user32.dll", EntryPoint = "CallWindowProcW")]
    private static extern nint CallWindowProc(nint lpPrevWndFunc, nint hWnd, uint msg, nint wParam, nint lParam);

    /// <summary>
    /// Show the global AI command palette with context from the current view.
    /// </summary>
    private void ShowAiCommandPalette()
    {
        // Gather source text: prefer selected text from the composer box,
        // then fall back to the current note content.
        var sourceText = ComposerBox.SelectedText;
        if (string.IsNullOrWhiteSpace(sourceText))
        {
            sourceText = ComposerBox.Text;
        }

        AiCommandPaletteControl.SourceText = sourceText?.Trim() ?? string.Empty;
        AiCommandPaletteControl.ContextNoteId = null;
        AiCommandPaletteControl.Show();
    }

    /// <summary>
    /// Show the Quick Ask overlay for a one-shot AI question.
    /// </summary>
    private void ShowQuickAsk()
    {
        QuickAskControl.Show();
    }

    /// <summary>
    /// Called when the user requests to insert an AI action result into the chat composer.
    /// </summary>
    private void OnPaletteInsertToChat(object? sender, string result)
    {
        DispatcherQueue.TryEnqueue(() =>
        {
            if (!string.IsNullOrWhiteSpace(result))
            {
                // If the composer already has text, append the result
                if (!string.IsNullOrWhiteSpace(ComposerBox.Text))
                {
                    ComposerBox.Text += "\n\n" + result;
                }
                else
                {
                    ComposerBox.Text = result;
                }
                ComposerBox.Focus(FocusState.Programmatic);
                ComposerBox.SelectionStart = ComposerBox.Text.Length;
            }
        });
    }

    /// <summary>
    /// Called when the user requests to insert the Quick Ask answer into the note editor.
    /// </summary>
    private void OnQuickAskInsertToNote(object? sender, string answer)
    {
        DispatcherQueue.TryEnqueue(() =>
        {
            if (!string.IsNullOrWhiteSpace(answer))
            {
                // If the composer already has text, append the answer
                if (!string.IsNullOrWhiteSpace(ComposerBox.Text))
                {
                    ComposerBox.Text += "\n\n" + answer;
                }
                else
                {
                    ComposerBox.Text = answer;
                }
                ComposerBox.Focus(FocusState.Programmatic);
                ComposerBox.SelectionStart = ComposerBox.Text.Length;
            }
        });
    }

    private void ScrollToLatest()
    {
        DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
        {
            ChatScrollViewer.UpdateLayout();
            ChatScrollViewer.ChangeView(null, ChatScrollViewer.ScrollableHeight, null, disableAnimation: false);
            JumpLatestButton.Visibility = Visibility.Collapsed;
        });
    }

    private void RefreshJumpLatestButton()
    {
        var canScroll = ChatScrollViewer.ScrollableHeight > 0;
        var awayFromLatest = ChatScrollViewer.VerticalOffset < ChatScrollViewer.ScrollableHeight - 32;
        JumpLatestButton.Visibility = canScroll && awayFromLatest
            ? Visibility.Visible
            : Visibility.Collapsed;
    }

    private void RefreshVaultSummary()
    {
        NotesText.Text = $"笔记：{_noteCount}";
    }

    // ── Note title map management (#2035) ──

    /// <summary>
    /// Loads the note title map (title -> noteId) from the Rust backend via listNotes.
    /// Results are cached with a 30-second TTL to avoid excessive backend calls.
    /// Titles are stored as-is (preserving original casing) for rendering display.
    /// </summary>
    private async Task<Dictionary<string, string>> LoadNoteTitleMapAsync()
    {
        // Check cache TTL
        var now = DateTime.UtcNow;
        if (_noteTitleMap is not null && (now - _noteTitleMapTimestamp).TotalMilliseconds < NoteTitleMapTtlMs)
        {
            return _noteTitleMap;
        }

        try
        {
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(15));
            var notes = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { }, cts.Token);
            if (notes is null || notes.Count == 0)
            {
                _noteTitleMap = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
                _noteTitleMapTimestamp = now;
                return _noteTitleMap;
            }

            // Build map: title -> id (case-insensitive keys)
            var map = new Dictionary<string, string>(notes.Count, StringComparer.OrdinalIgnoreCase);
            foreach (var note in notes)
            {
                if (!string.IsNullOrWhiteSpace(note.Title) && !map.ContainsKey(note.Title))
                {
                    map[note.Title] = note.Id;
                }
            }

            _noteTitleMap = map;
            _noteTitleMapTimestamp = now;
            return map;
        }
        catch (Exception error)
        {
            System.Diagnostics.Debug.WriteLine($"[LoadNoteTitleMapAsync] Failed: {error.Message}");
            // Return cached map (if any) on failure
            return _noteTitleMap ?? new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        }
    }

    /// <summary>
    /// Forces the note title cache to be refreshed on the next call to LoadNoteTitleMapAsync.
    /// Called after tool actions that modify notes (create, delete, rename).
    /// </summary>
    private void InvalidateNoteTitleCache()
    {
        _noteTitleMapTimestamp = DateTime.MinValue;
    }

    /// <summary>
    /// Navigates to a note by looking up its title in the note title map.
    /// Falls back to treating the parameter as a noteId if no title match is found.
    /// Called when a [[wikilink]] or auto-detected note reference is clicked.
    /// </summary>
    private async Task NavigateToNoteFromTitleAsync(string? noteTitleOrId)
    {
        if (string.IsNullOrWhiteSpace(noteTitleOrId))
            return;

        try
        {
            // Resolve the destination note ID first (#3239)
            var titleMap = await LoadNoteTitleMapAsync();
            var noteId = titleMap.TryGetValue(noteTitleOrId, out var id)
                ? id
                : noteTitleOrId; // fallback: treat as id directly

            // Record the DESTINATION in the navigation stack so the browse
            // trace is canonical: every entry is a note the user visited TO.
            // Compare against the resolved noteId, not the raw input, so a
            // wikilink that uses the note's title while already on that note
            // does not push a self-loop (#3239, #3230).
            var currentNoteId = _notesView?.SelectedNoteId();
            if (currentNoteId is not null && currentNoteId != noteId)
            {
                // Truncate forward history when navigating from a non-tip position
                if (_noteNavIndex >= 0 && _noteNavIndex < _noteNavStack.Count - 1)
                    _noteNavStack.RemoveRange(_noteNavIndex + 1, _noteNavStack.Count - (_noteNavIndex + 1));
                _noteNavStack.Add(noteId);
                _noteNavIndex = _noteNavStack.Count - 1;
            }

            // Navigate to the Notes view
            await SwitchToNotesViewAsync();

            // Give the UI a moment to load the NotesView, then select the note
            await Task.Delay(100);

            if (_notesView is not null)
            {
                await _notesView.RefreshNotesAsync();
                _notesView.SelectNoteById(noteId);
            }
        }
        catch (Exception error)
        {
            System.Diagnostics.Debug.WriteLine($"[NavigateToNoteFromTitleAsync] Error: {error.Message}");
            ShowError("打开笔记失败", error);
        }
    }

    private void OnNavigateBack(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        if (_noteNavIndex <= 0 || _notesView is null) return;
        _noteNavIndex--;
        var noteId = _noteNavStack[_noteNavIndex];
        _ = NavigateToNoteFromHistoryAsync(noteId);
        args.Handled = true;
    }

    private void OnNavigateForward(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        if (_noteNavIndex >= _noteNavStack.Count - 1 || _notesView is null) return;
        _noteNavIndex++;
        var noteId = _noteNavStack[_noteNavIndex];
        _ = NavigateToNoteFromHistoryAsync(noteId);
        args.Handled = true;
    }

    /// <summary>
    /// Navigates to a note from the history stack without recording it again.
    /// (#3230 — keeps the browse history intact, like browser back/forward)
    /// </summary>
    private async Task NavigateToNoteFromHistoryAsync(string noteId)
    {
        try
        {
            await SwitchToNotesViewAsync();
            await Task.Delay(100);
            if (_notesView is not null)
            {
                await _notesView.RefreshNotesAsync();
                _notesView.SelectNoteById(noteId);
            }
        }
        catch (Exception error)
        {
            System.Diagnostics.Debug.WriteLine($"[NavigateToNoteFromHistoryAsync] Error: {error.Message}");
        }
    }
}
