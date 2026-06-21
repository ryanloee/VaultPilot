using VaultPilot.WinUI.Backend;
using VaultPilot.WinUI.Controls;
using VaultPilot.WinUI.Models;
using VaultPilot.WinUI.Views;
using Microsoft.UI.Input;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Text;
using System.Text.RegularExpressions;
using System.Linq;
using System.Threading;
using Windows.ApplicationModel.DataTransfer;
using Windows.Foundation;
using Windows.Graphics;
using Windows.Storage;
using Windows.Storage.Pickers;
using Windows.Storage.Streams;
using Windows.System;
using WinRT.Interop;

namespace VaultPilot.WinUI;

public sealed partial class MainWindow : Window
{
    private const double ContextCompressionThreshold = 0.95;
    private const int RecentTurnsAfterCompression = 8;
    private const ulong ImageAttachmentTokenEstimate = 1200;
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
    private const string MarkdownOpenTag = "<vp-markdown>";
    private const string MarkdownCloseTag = "</vp-markdown>";
    private readonly BackendClient _backendClient;
    private AppWindow? _appWindow;
    // Theme-aware status brushes — looked up from ThemeResource each call so they
    // automatically track theme changes (light/dark).  The dictionary lookup is O(1).
    private static Brush BrushRed => GetThemeBrush("StatusErrorBrush");
    private static Brush BrushOrange => GetThemeBrush("StatusWarningBrush");
    private static Brush BrushGreen => GetThemeBrush("StatusSuccessBrush");
    private static Brush BrushLimeGreen => GetThemeBrush("StatusSuccessBrush");
    // Code block and attachment colors now use theme-aware ThemeResource brushes
    // defined in App.xaml ThemeDictionaries (see #196)

    /// <summary>Looks up a theme-aware brush from application resources.</summary>
    private static readonly SolidColorBrush _transparentBrush = new(Microsoft.UI.Colors.Transparent);

    private static Brush GetThemeBrush(string key)
    {
        if (Application.Current?.Resources is not null
            && Application.Current.Resources.TryGetValue(key, out var value) && value is Brush brush)
        {
            return brush;
        }
        System.Diagnostics.Debug.WriteLine($"[GetThemeBrush] Missing resource key: '{key}', falling back to Transparent.");
        return _transparentBrush;
    }
    /// <summary>Looks up a theme-aware Style from application resources, returning null if missing.</summary>
    private static Style? GetThemeStyle(string key)
    {
        if (Application.Current?.Resources is not null
            && Application.Current.Resources.TryGetValue(key, out var value) && value is Style style)
            return style;
        System.Diagnostics.Debug.WriteLine($"[GetThemeStyle] Missing resource key: '{key}'.");
        return null;
    }
    private ChatState _chatState = new(string.Empty, Array.Empty<ChatSession>());
    private readonly SemaphoreSlim _chatStateLock = new(1, 1);
    private string _currentSessionId = string.Empty;
    private AppSettings? _settings;
    private int _noteCount;
    private readonly List<ChatAttachment> _attachments = [];
    private bool _sidebarCollapsed = true;
    private bool _sidebarAutoCollapsed = true;
    private double _contextUsagePercent;
    private string _startupStep = "初始化";
    private volatile int _updateDownloadPercent = -1;
    private volatile string _updateDownloadVersion = string.Empty;
    private DispatcherTimer? _autoWakeTimer;
    private Views.NotesView? _notesView;
    private bool _notesViewLoaded;
    private nint _windowHandle;
    private nint _originalWindowProc;
    private WindowProcDelegate? _windowProcDelegate;
    private GCHandle _windowProcDelegateHandle;
    private FrameworkElement? _thinkingIndicator;
    private DispatcherTimer? _thinkingDotsTimer;
    private int _thinkingDotStep;
    private CancellationTokenSource? _activeRequestCts;
    private volatile Task? _activeRequestTask;
    private int _requestInProgress; // #676: guard against concurrent ExecuteAiRequestAsync calls

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
        ChatScrollViewer.ViewChanged += OnChatScrollViewerViewChanged;
        JumpLatestButton.Click += OnJumpLatestClicked;
        RootGrid.SizeChanged += OnRootGridSizeChanged;

        // KeyboardAccelerators for keys that the WinUI XamlCompiler cannot parse (OemComma, Number1, Number2)
        AddKeyboardAccelerator((VirtualKey)188 /* OemComma */, VirtualKeyModifiers.Control, OnSettingsAccelerator);
        AddKeyboardAccelerator(VirtualKey.Number1, VirtualKeyModifiers.Control, OnNavChatAccelerator);
        AddKeyboardAccelerator(VirtualKey.Number2, VirtualKeyModifiers.Control, OnNavNotesAccelerator);
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
            await LogStartup("Window loaded");
            await UpdateStartupStepAsync("启动后端");
            var backendPath = ResolveBackendPath();
            await LogStartup($"Backend path: {backendPath}");
            _backendClient.Start(backendPath);
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
            AppendMessage("错误", $"聊天记录读取失败，已使用空会话：{LocalizeError(error.Message)}");
            return new ChatState(string.Empty, Array.Empty<ChatSession>());
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

            var models = GetModelsForProvider(_settings.Provider.BaseUrl);

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

    private async void OnSendClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            await SendCurrentMessageAsync();
        }
        catch (Exception error)
        {
            ShowError("发送消息失败", error);
        }
    }

    private async void OnRecordClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            await RecordCurrentMessageAsync();
        }
        catch (Exception error)
        {
            ShowError("录音消息失败", error);
        }
    }

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

    private void OnChatScrollViewerViewChanged(object? sender, ScrollViewerViewChangedEventArgs e)
    {
        RefreshJumpLatestButton();
    }

    private void OnJumpLatestClicked(object sender, RoutedEventArgs e)
    {
        ScrollToLatest();
    }

    private void OnToggleSidebarClicked(object sender, RoutedEventArgs e)
    {
        SetSidebarCollapsed(!_sidebarCollapsed, autoCollapsed: false);
    }

    private async void OnNavigationSelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        try
        {
            if (args.SelectedItem is not NavigationViewItem item || item.Tag is not string tag)
            {
                return;
            }

            switch (tag)
            {
                case "Chat":
                    ChatView.Visibility = Visibility.Visible;
                    NotesViewHost.Visibility = Visibility.Collapsed;
                    break;

                case "Notes":
                    ChatView.Visibility = Visibility.Collapsed;
                    NotesViewHost.Visibility = Visibility.Visible;
                    if (!_notesViewLoaded)
                    {
                        _notesView = new Views.NotesView(_backendClient);
                        NotesViewHost.Children.Add(_notesView);
                        _notesViewLoaded = true;
                    }
                    await _notesView.RefreshNotesAsync();
                    break;
            }
        }
        catch (Exception error)
        {
            System.Diagnostics.Debug.WriteLine($"[OnNavigationSelectionChanged] Error: {error}");
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
        SidebarColumn.Width = collapsed ? new GridLength(0) : new GridLength(280);
        ContentGrid.ColumnSpacing = collapsed ? 0 : 16;
        ToggleSidebarButton.Label = collapsed ? "展开会话" : "收起会话";
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
            _attachments.Clear();
            ComposerBox.Text = string.Empty;
            RenderCurrentSession();
            RefreshAttachments();
            RefreshSessions();
            await SaveChatStateAsync();

            UpdateStatusBar("success", "已新建对话", "可以开始新的提问。");
        }
        catch (Exception error)
        {
            ShowError("新建会话失败", error);
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

    private TextBlock? _composerMeasureBlock;

    private void OnComposerTextChanged(object sender, TextChangedEventArgs e)
    {
        var textBox = (TextBox)sender;
        if (textBox.ActualWidth <= 0) return;

        _composerMeasureBlock ??= new TextBlock
        {
            FontFamily = textBox.FontFamily,
            FontSize = textBox.FontSize,
            FontWeight = textBox.FontWeight,
            TextWrapping = TextWrapping.Wrap,
        };

        _composerMeasureBlock.Text = textBox.Text ?? string.Empty;
        var availableWidth = textBox.ActualWidth - 20; // padding + scrollbar
        _composerMeasureBlock.Measure(new Windows.Foundation.Size(availableWidth, double.PositiveInfinity));

        var desiredHeight = _composerMeasureBlock.DesiredSize.Height + 20; // inner padding
        var clampedHeight = Math.Max(88, Math.Min(200, desiredHeight));
        textBox.Height = clampedHeight;
    }

    private async void OnComposerKeyDown(object sender, KeyRoutedEventArgs e)
    {
        try
        {
            if (e.Key == VirtualKey.V)
            {
                var controlState = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Control);
                if (controlState.HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down))
                {
                    // #805: Only suppress default paste when clipboard has no text content.
                    // When text is available, prefer default paste to avoid text loss
                    // when StorageItems contain no images (regression from #627 fix).
                    bool suppressForImagePaste = true;
                    try
                    {
                        var content = Clipboard.GetContent();
                        suppressForImagePaste = content?.Contains(StandardDataFormats.Text) != true;
                    }
                    catch { /* clipboard access can fail; default to image paste attempt */ }

                    if (suppressForImagePaste)
                    {
                        // Set Handled pre-emptively to block the default paste handler
                        // during the await. Reset if image paste doesn't apply. (#627)
                        e.Handled = true;
                        if (await TryHandleClipboardImagePasteAsync())
                        {
                            return;
                        }
                        e.Handled = false;
                    }
                }
            }

            if (e.Key != VirtualKey.Enter)
            {
                return;
            }

            var shiftState = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift);
            if (shiftState.HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down))
            {
                // #859: AcceptsReturn is false so we manually insert a newline
                // at the cursor position for Shift+Enter.
                var cursorPos = ComposerBox.SelectionStart;
                ComposerBox.Text = ComposerBox.Text.Insert(cursorPos, Environment.NewLine);
                ComposerBox.SelectionStart = cursorPos + Environment.NewLine.Length;
                e.Handled = true;
                return;
            }

            e.Handled = true;
            await SendCurrentMessageAsync();
        }
        catch (Exception error)
        {
            ShowError("键盘事件处理失败", error);
        }
    }

    // Chat request flow, session management, rendering, and context management
    // are in MainWindow.Chat.cs (#1206).

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
        CancelActiveRequest();
    }

    private void OnNavChatAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        NavChat.IsSelected = true;
    }

    private void OnNavNotesAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        NavNotes.IsSelected = true;
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

    // PrepareExitAsync, ShutdownAsync, OnAgentStatusReceived, OnConnectionStateChanged
    // are in MainWindow.Chat.cs (#1206).

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

    // AppendMessage, ShowThinkingIndicator, RemoveThinkingIndicator
    // are in MainWindow.Chat.cs (#1206).


    private void CopyTextToClipboard(string text)
    {
        try
        {
            var package = new DataPackage();
            package.SetText(text);
            Clipboard.SetContent(package);
            Clipboard.Flush();
            UpdateStatusBar("success", "已复制", "消息内容已复制到剪贴板。");
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"Clipboard copy failed: {ex.Message}");
            UpdateStatusBar("warning", "复制失败", "无法写入剪贴板，可能被其他程序占用。");
        }
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
            using var cts = new CancellationTokenSource(
                TimeSpan.FromMilliseconds((_settings?.Provider.RequestTimeoutMs ?? 60_000) + 30_000));
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
            await LogStartup("自动唤醒完成: 已发送提问并收到回复");
        }
        catch (Exception error)
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

    // ScrollToLatest, RefreshJumpLatestButton, RefreshVaultSummary
    // are in MainWindow.Chat.cs (#1206).
    private static string ResolveDisplayVersion()
    {
        var informationalVersion = typeof(MainWindow).Assembly
            .GetCustomAttribute<AssemblyInformationalVersionAttribute>()?
            .InformationalVersion;
        var cleanVersion = (informationalVersion ?? string.Empty).Split('+', 2)[0].Trim();
        if (string.IsNullOrWhiteSpace(cleanVersion))
        {
            cleanVersion = typeof(MainWindow).Assembly.GetName().Version?.ToString() ?? "0.0.0";
        }

        return cleanVersion.StartsWith("v", StringComparison.OrdinalIgnoreCase)
            ? cleanVersion
            : $"v{cleanVersion}";
    }


    // ShowError, ShowStartupFailureAsync, SendWithTimeoutAsync, UpdateStartupStepAsync,
    // UpdateStatusBar, RestoreIdleStatus, ShowLoadingOverlay, HideLoadingOverlay,
    // StartupLogPath, LogStartup, EnsureCurrentSession, RenderCurrentSession,
    // ShowEmptyState, AppendThinkingTrace, AppendCitationCards, CurrentSession,
    // FindSessionById, GetConversationHistory, CompressCurrentSessionIfNeededAsync,
    // RefreshContextStatus, EstimateSessionTokens, EstimateTurnTokens, EstimateTokensForText,
    // ResolveContextWindowTokens, ContainsModelToken, IsOpenAiOSeriesModel, IsModelSeparator,
    // FormatTokenCount, AddTurnAsync, SaveChatStateAsync, RefreshSessions,
    // ToRelativeTime, BuildSessionTitle, LocalizeStage, LocalizeStatusDetail,
    // LocalizeError, SessionListItem
    // are in MainWindow.Chat.cs (#1206).
}
