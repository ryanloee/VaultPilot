using VaultPilot.WinUI.Backend;
using VaultPilot.WinUI.Controls;
using VaultPilot.WinUI.Models;
using Microsoft.UI.Input;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
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
    private const double SettingsDialogWidth = 520;
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
    // Cached brushes to avoid per-call allocations (see #130)
    private static readonly SolidColorBrush BrushRed = new(Microsoft.UI.Colors.Red);
    private static readonly SolidColorBrush BrushOrange = new(Microsoft.UI.Colors.Orange);
    private static readonly SolidColorBrush BrushGreen = new(Microsoft.UI.Colors.Green);
    private static readonly SolidColorBrush BrushLimeGreen = new(Microsoft.UI.Colors.LimeGreen);
    // Code block and attachment colors now use theme-aware ThemeResource brushes
    // defined in App.xaml ThemeDictionaries (see #196)

    /// <summary>Looks up a theme-aware brush from application resources.</summary>
    private static Brush GetThemeBrush(string key)
    {
        return (Brush)Application.Current.Resources[key];
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
    private string _updateDownloadVersion = string.Empty;
    private DispatcherTimer? _autoWakeTimer;
    private nint _windowHandle;
    private nint _originalWindowProc;
    private WindowProcDelegate? _windowProcDelegate;
    private FrameworkElement? _thinkingIndicator;
    private DispatcherTimer? _thinkingDotsTimer;
    private int _thinkingDotStep;
    private CancellationTokenSource? _activeRequestCts;

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
    }

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        try
        {
            LogStartup("Window loaded");
            UpdateStartupStep("启动后端");
            var backendPath = ResolveBackendPath();
            LogStartup($"Backend path: {backendPath}");
            _backendClient.Start(backendPath);
            LogStartup("Backend process started");
            UpdateStartupStep("检查后端响应");
            await SendWithTimeoutAsync(
                (token) => _backendClient.SendAsync("ping", new { }, token),
                "ping");
            LogStartup("Ping ok");

            UpdateStartupStep("读取设置");
            _settings = await SendWithTimeoutAsync(
                (token) => _backendClient.SendAsync<AppSettings>("getSettings", new { }, token),
                "getSettings");

            UpdateStartupStep("读取聊天记录");
            _chatState = await TryLoadChatStateAsync();

            UpdateStartupStep("读取笔记列表");
            _noteCount = await TryLoadNoteCountAsync();
            EnsureCurrentSession();

            RefreshVaultSummary();
            RefreshSessions();
            SetSidebarCollapsed(collapsed: true, autoCollapsed: true);
            RenderCurrentSession();
            ScrollToLatest();

            UpdateStatusBar("success", "后端已连接", "就绪");
            LogStartup("Startup complete");
            ApplyAutoWakeSettings();
            ShowNextWakeTime();
            if (_settings?.AutoCheckUpdates ?? true)
            {
                _ = CheckForAppUpdatesAsync();
            }
            else
            {
                LogStartup("Update check skipped: disabled in settings.");
            }
        }
        catch (Exception error)
        {
            await ShowStartupFailureAsync(error, _backendClient.GetStderrTail());
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
            _settings ??= await _backendClient.SendAsync<AppSettings>("getSettings", new { });
            if (_settings is null)
            {
                throw new InvalidOperationException("设置加载失败。");
            }

            var vaultBox = new TextBox
            {
                Header = "知识库目录",
                Text = _settings.VaultDir,
                PlaceholderText = "例如 D:\\KnowledgeVault",
                HorizontalAlignment = HorizontalAlignment.Stretch
            };
            var openVaultButton = new Button
            {
                Content = "打开目录"
            };
            openVaultButton.Click += async (_, _) => await OpenVaultDirectoryAsync();
            var apiKeyBox = new PasswordBox
            {
                Header = "API Key",
                Password = _settings.Provider.ApiKey,
                PlaceholderText = "输入模型服务 API Key",
                HorizontalAlignment = HorizontalAlignment.Stretch
            };
            var baseUrlBox = new TextBox
            {
                Header = "接口地址",
                Text = _settings.Provider.BaseUrl,
                PlaceholderText = "例如 https://api.openai.com/v1",
                HorizontalAlignment = HorizontalAlignment.Stretch
            };
            var modelBox = new TextBox
            {
                Header = "模型",
                Text = _settings.Provider.Model,
                PlaceholderText = "例如 gpt-4o-mini",
                HorizontalAlignment = HorizontalAlignment.Stretch
            };
            var timeoutBox = new TextBox
            {
                Header = "请求超时（毫秒）",
                Text = _settings.Provider.RequestTimeoutMs.ToString(),
                HorizontalAlignment = HorizontalAlignment.Stretch
            };
            var contextWindowBox = new TextBox
            {
                Header = "上下文窗口 Token 数（可选）",
                Text = _settings.Provider.ContextWindowTokens?.ToString() ?? string.Empty,
                HorizontalAlignment = HorizontalAlignment.Stretch
            };
            var autoCheckUpdatesBox = new CheckBox
            {
                Content = "启动时自动检查更新",
                IsChecked = _settings.AutoCheckUpdates,
                HorizontalAlignment = HorizontalAlignment.Left
            };

            // Auto-wake section.
            var autoWakeSeparator = new Border
            {
                Height = 1,
                Background = (Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"],
                Margin = new Thickness(0, 4, 0, 4),
            };
            var autoWakeHeader = new TextBlock
            {
                Text = "自动唤醒",
                FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            };
            var autoWakeEnabledBox = new CheckBox
            {
                Content = "启用自动唤醒（定时调用 API 保持会话活跃）",
                IsChecked = _settings.AutoWakeEnabled,
                HorizontalAlignment = HorizontalAlignment.Left,
            };
            var autoWakeIntervalBox = new TextBox
            {
                Header = "唤醒间隔（分钟）",
                Text = _settings.AutoWakeIntervalMinutes.ToString(),
                PlaceholderText = "30",
                HorizontalAlignment = HorizontalAlignment.Stretch,
            };
            var autoWakeModelBox = new ComboBox
            {
                Header = "唤醒使用的模型（留空使用默认模型）",
                HorizontalAlignment = HorizontalAlignment.Stretch,
                IsEditable = true,
                PlaceholderText = _settings.Provider.Model,
            };
            autoWakeModelBox.Items.Add(string.Empty);
            foreach (var model in GetModelsForProvider(_settings.Provider.BaseUrl))
            {
                autoWakeModelBox.Items.Add(model);
            }
            if (string.IsNullOrEmpty(_settings.AutoWakeModel))
            {
                autoWakeModelBox.SelectedIndex = 0;
            }
            else
            {
                autoWakeModelBox.Text = _settings.AutoWakeModel;
            }
            var autoWakeStartTimeBox = new TextBox
            {
                Header = "开始时间（HH:mm，留空不限）",
                Text = _settings?.AutoWakeStartTime ?? string.Empty,
                PlaceholderText = "05:00",
                HorizontalAlignment = HorizontalAlignment.Stretch,
            };
            var autoWakeEndTimeBox = new TextBox
            {
                Header = "结束时间（HH:mm，留空不限）",
                Text = _settings?.AutoWakeEndTime ?? string.Empty,
                PlaceholderText = "23:00",
                HorizontalAlignment = HorizontalAlignment.Stretch,
            };

            var projectLinkButton = new Button
            {
                Content = "项目地址",
                HorizontalAlignment = HorizontalAlignment.Left
            };
            projectLinkButton.Click += async (_, _) => await OpenProjectHomepageAsync();
            var versionLabel = new TextBlock
            {
                Text = ResolveDisplayVersion(),
                Opacity = 0.6,
                VerticalAlignment = VerticalAlignment.Center,
                HorizontalAlignment = HorizontalAlignment.Right
            };
            var footerRow = new Grid
            {
                ColumnSpacing = 12
            };
            footerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            footerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            footerRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            Grid.SetColumn(autoCheckUpdatesBox, 0);
            Grid.SetColumn(projectLinkButton, 1);
            Grid.SetColumn(versionLabel, 2);
            footerRow.Children.Add(autoCheckUpdatesBox);
            footerRow.Children.Add(projectLinkButton);
            footerRow.Children.Add(versionLabel);

            var panel = new StackPanel
            {
                Spacing = 12,
                Width = SettingsDialogWidth,
                HorizontalAlignment = HorizontalAlignment.Stretch
            };
            panel.Children.Add(vaultBox);
            panel.Children.Add(openVaultButton);
            panel.Children.Add(apiKeyBox);
            panel.Children.Add(baseUrlBox);
            panel.Children.Add(modelBox);
            panel.Children.Add(timeoutBox);
            panel.Children.Add(contextWindowBox);
            panel.Children.Add(autoWakeSeparator);
            panel.Children.Add(autoWakeHeader);
            panel.Children.Add(autoWakeEnabledBox);
            panel.Children.Add(autoWakeIntervalBox);
            panel.Children.Add(autoWakeModelBox);
            panel.Children.Add(autoWakeStartTimeBox);
            panel.Children.Add(autoWakeEndTimeBox);

            var nextWakeLabel = new TextBlock
            {
                Opacity = 0.7,
            };
            if (_settings?.AutoWakeEnabled == true)
            {
                var next = GetNextAutoWakeTime();
                if (next.HasValue)
                {
                    nextWakeLabel.Text = next.Value.Date == DateTime.Today
                        ? $"下次唤醒: {next.Value:HH:mm}"
                        : $"下次唤醒: {next.Value:MM/dd HH:mm}";
                }
            }
            panel.Children.Add(nextWakeLabel);

            // Inline error bar shown at the top of the dialog.
            var errorInfoBar = new InfoBar
            {
                Severity = InfoBarSeverity.Error,
                Title = "设置校验失败",
                IsOpen = false,
                IsClosable = false,
                HorizontalAlignment = HorizontalAlignment.Stretch,
            };
            panel.Children.Insert(0, errorInfoBar);

            panel.Children.Add(footerRow);

            var dialog = new ContentDialog
            {
                XamlRoot = RootGrid.XamlRoot,
                Title = "设置",
                PrimaryButtonText = "保存",
                CloseButtonText = "取消",
                DefaultButton = ContentDialogButton.Primary,
                Content = new ScrollViewer
                {
                    MaxHeight = 520,
                    HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
                    Content = panel
                }
            };

            // Validate and save BEFORE the dialog closes so the user never
            // loses input on a validation failure.  Setting args.Cancel = true
            // keeps the dialog open; only an error-free path lets it close.
            dialog.PrimaryButtonClick += async (_, args) =>
            {
                var deferral = args.GetDeferral();
                try
                {
                    var validationErrors = new List<string>();

                    var trimmedApiKey = apiKeyBox.Password.Trim();
                    if (string.IsNullOrEmpty(trimmedApiKey))
                    {
                        validationErrors.Add("API Key 不能为空。");
                    }

                    var trimmedBaseUrl = baseUrlBox.Text.Trim();
                    if (string.IsNullOrEmpty(trimmedBaseUrl))
                    {
                        validationErrors.Add("接口地址不能为空。");
                    }
                    else if (!Uri.TryCreate(trimmedBaseUrl, UriKind.Absolute, out var parsedUri)
                             || (parsedUri.Scheme != "http" && parsedUri.Scheme != "https"))
                    {
                        validationErrors.Add("接口地址必须是有效的 http:// 或 https:// URL。");
                    }

                    var trimmedModel = modelBox.Text.Trim();
                    if (string.IsNullOrEmpty(trimmedModel))
                    {
                        validationErrors.Add("模型名称不能为空。");
                    }

                    var trimmedWakeStart = autoWakeStartTimeBox.Text?.Trim() ?? string.Empty;
                    if (!string.IsNullOrEmpty(trimmedWakeStart) && !TimeSpan.TryParse(trimmedWakeStart, out _))
                    {
                        validationErrors.Add("自动唤醒开始时间格式无效，请使用 HH:mm 格式。");
                    }

                    var trimmedWakeEnd = autoWakeEndTimeBox.Text?.Trim() ?? string.Empty;
                    if (!string.IsNullOrEmpty(trimmedWakeEnd) && !TimeSpan.TryParse(trimmedWakeEnd, out _))
                    {
                        validationErrors.Add("自动唤醒结束时间格式无效，请使用 HH:mm 格式。");
                    }

                    if (!ulong.TryParse(timeoutBox.Text.Trim(), out var timeoutMs) || timeoutMs == 0)
                    {
                        validationErrors.Add("请求超时必须是大于 0 的数字。");
                    }

                    ulong? contextWindowTokens = null;
                    if (!string.IsNullOrWhiteSpace(contextWindowBox.Text))
                    {
                        if (!ulong.TryParse(contextWindowBox.Text.Trim(), out var parsedContextWindow))
                        {
                            validationErrors.Add("上下文窗口 Token 数必须是数字。");
                        }
                        else
                        {
                            contextWindowTokens = parsedContextWindow;
                        }
                    }

                    if (validationErrors.Count > 0)
                    {
                        errorInfoBar.Message = string.Join("\n", validationErrors);
                        errorInfoBar.IsOpen = true;
                        args.Cancel = true;
                        return;
                    }

                    errorInfoBar.IsOpen = false;

                    if (!ulong.TryParse(autoWakeIntervalBox.Text?.Trim() ?? "30", out var autoWakeInterval) || autoWakeInterval == 0)
                    {
                        autoWakeInterval = 30;
                    }

                    var autoWakeModel = (autoWakeModelBox.SelectedItem as string ?? autoWakeModelBox.Text ?? string.Empty).Trim();
                    var autoWakeStartTime = trimmedWakeStart;
                    var autoWakeEndTime = trimmedWakeEnd;

                    var updated = new AppSettings(
                        vaultBox.Text.Trim(),
                        new ProviderConfig(
                            trimmedApiKey,
                            trimmedBaseUrl,
                            trimmedModel,
                            timeoutMs,
                            contextWindowTokens),
                        autoCheckUpdatesBox.IsChecked ?? true,
                        autoWakeEnabledBox.IsChecked ?? false,
                        autoWakeInterval,
                        autoWakeModel,
                        autoWakeStartTime,
                        autoWakeEndTime);

                    _settings = await _backendClient.SendAsync<AppSettings>("saveSettings", new { settings = updated });
                    RefreshVaultSummary();
                    RefreshContextStatus();
                    ApplyAutoWakeSettings();
                    UpdateStatusBar("success", "设置已保存", "模型服务配置已更新。");
                    ShowNextWakeTime();
                }
                catch (Exception error)
                {
                    ShowError("保存设置失败", error);
                    args.Cancel = true;
                }
                finally
                {
                    deferral.Complete();
                }
            };

            await dialog.ShowAsync();
        }
        catch (Exception error)
        {
            ShowError("保存设置失败", error);
        }
    }

    private async void OnRebuildClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            RebuildButton.IsEnabled = false;
            UpdateStatusBar("info", "正在重建索引", "正在扫描知识库...");

            var stats = await _backendClient.SendAsync<IndexStats>("rebuildIndex", new { });
            var notes = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { });
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
            var result = await _backendClient.SendAsync<ImportResult>("importMarkdown", new { paths });
            var notes = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { });
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
            var launched = Process.Start(new ProcessStartInfo
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
                if (controlState.HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down)
                    && await TryHandleClipboardImagePasteAsync())
                {
                    e.Handled = true;
                    return;
                }
            }

            if (e.Key != VirtualKey.Enter)
            {
                return;
            }

            var shiftState = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift);
            if (shiftState.HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down))
            {
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

    private async Task SendCurrentMessageAsync()
    {
        if (!SendButton.IsEnabled)
        {
            return;
        }

        var text = ComposerBox.Text.Trim();
        if (string.IsNullOrEmpty(text) && _attachments.Count == 0)
        {
            return;
        }

        var pendingAttachments = _attachments.ToArray();
        var prompt = string.IsNullOrWhiteSpace(text)
            ? "请结合我发送的图片理解并回复。"
            : text;
        var userDisplay = string.IsNullOrWhiteSpace(text)
            ? "（发送了一张图片）"
            : text;

        await ExecuteAiRequestAsync(
            prompt, userDisplay, pendingAttachments, text,
            "助手处理中", "正在准备请求...", "请求失败");

        if (_lastAiAnswer?.SavedNote is not null)
        {
            AppendMessage("系统", $"已保存笔记：{_lastAiAnswer.SavedNote.Title}");
            ScrollToLatest();
        }

        RestoreIdleStatus();
    }

    private async Task RecordCurrentMessageAsync()
    {
        if (!RecordButton.IsEnabled)
        {
            return;
        }

        var text = ComposerBox.Text.Trim();
        var pendingAttachments = _attachments.ToArray();

        if (string.IsNullOrEmpty(text) && pendingAttachments.Length == 0)
        {
            var session = CurrentSession();
            var lastAssistantTurn = session?.Turns.LastOrDefault(t => t.Role == "assistant");
            if (lastAssistantTurn is null)
            {
                return;
            }

            text = $"请将刚才讨论的内容整理记录到知识库";
        }

        var prompt = $"请将以下内容记录到知识库：{text}";
        var userDisplay = string.IsNullOrWhiteSpace(ComposerBox.Text)
            ? "（记录了当前对话内容）"
            : ComposerBox.Text.Trim();

        await ExecuteAiRequestAsync(
            prompt, userDisplay, pendingAttachments, text,
            "正在记录知识", "正在整理并保存...", "记录失败",
            passCancellationToken: true);

        if (_lastAiAnswer?.SavedNote is null)
        {
            throw new InvalidOperationException("知识库写入未完成，模型未返回已保存笔记。");
        }

        var savedNote = _lastAiAnswer.SavedNote;
        AppendMessage("系统", $"已保存笔记：{savedNote.Title}");
        ScrollToLatest();
        var notes = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { });
        _noteCount = notes?.Count ?? 0;
        RefreshVaultSummary();

        RestoreIdleStatus("知识已记录", $"已保存为笔记：{savedNote.Title}");
    }

    /// <summary>
    /// Shared implementation for Send and Record flows: clears the composer,
    /// sends the prompt to the AI backend, and updates the session.
    /// </summary>
    private GroundedAnswer? _lastAiAnswer;

    private async Task ExecuteAiRequestAsync(
        string prompt,
        string userDisplay,
        AttachmentItem[] pendingAttachments,
        string originalText,
        string statusTitle,
        string statusDetail,
        string errorTitle,
        bool passCancellationToken = false)
    {
        ComposerBox.Text = string.Empty;
        _attachments.Clear();
        RefreshAttachments();

        _activeRequestCts?.Dispose();
        _activeRequestCts = new CancellationTokenSource();
        var cancellationToken = _activeRequestCts.Token;

        _lastAiAnswer = null;

        try
        {
            SendButton.IsEnabled = false;
            RecordButton.IsEnabled = false;
            CancelButton.Visibility = Visibility.Visible;
            UpdateStatusBar("info", statusTitle, statusDetail);

            await CompressCurrentSessionIfNeededAsync(prompt, pendingAttachments);
            var history = GetConversationHistory();
            AddTurn("user", userDisplay, attachments: pendingAttachments);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();

            ShowThinkingIndicator();
            ScrollToLatest();

            var answer = passCancellationToken
                ? await _backendClient.SendAsync<GroundedAnswer>(
                    "askWithAi",
                    new
                    {
                        question = prompt,
                        history,
                        imagePaths = pendingAttachments.Select(item => item.Path).ToArray()
                    },
                    cancellationToken)
                : await _backendClient.SendAsync<GroundedAnswer>(
                    "askWithAi",
                    new
                    {
                        question = prompt,
                        history,
                        imagePaths = pendingAttachments.Select(item => item.Path).ToArray()
                    });
            RemoveThinkingIndicator();
            _lastAiAnswer = answer;

            AddTurn("assistant", answer?.Answer ?? string.Empty, answer);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();
        }
        catch (Exception error)
        {
            RemoveThinkingIndicator();
            ComposerBox.Text = originalText;
            _attachments.AddRange(pendingAttachments);
            RefreshAttachments();
            var message = LocalizeError(error.Message);
            AddTurn("assistant", message);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();
            ShowError(errorTitle, error, addMessage: false);
        }
        finally
        {
            _activeRequestCts?.Dispose();
            _activeRequestCts = null;
            SendButton.IsEnabled = true;
            RecordButton.IsEnabled = true;
            CancelButton.Visibility = Visibility.Collapsed;
            RefreshSessions();
        }
    }

    private async void OnClosed(object sender, WindowEventArgs args)
    {
        try
        {
            _activeRequestCts?.Cancel();
            _activeRequestCts?.Dispose();
            _activeRequestCts = null;
            RemoveThinkingIndicator();
            UnsubscribeEvents();
            TryReleaseWindowFileDropHook();
            await _backendClient.DisposeAsync();
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
    }

    /// <summary>
    /// Cancels the currently active AI request, if any.
    /// Safe to call when no request is in progress.
    /// </summary>
    public void CancelActiveRequest()
    {
        _activeRequestCts?.Cancel();
    }

    #region Keyboard Accelerator Handlers

    private void OnNewSessionAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        _ = OnNewSessionClicked(NewSessionButton, new RoutedEventArgs());
    }

    private void OnToggleSidebarAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        OnToggleSidebarClicked(ToggleSidebarButton, new RoutedEventArgs());
    }

    private void OnSettingsAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        _ = OnSettingsClicked(SettingsButton, new RoutedEventArgs());
    }

    private void OnEscapeAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        CancelActiveRequest();
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
        RemoveThinkingIndicator();
        StopAutoWakeTimer();
        UnsubscribeEvents();
        TryReleaseWindowFileDropHook();
        await SaveChatStateAsync();
        await _backendClient.DisposeAsync();
        PruneClipboardImages();
    }

    private void OnAgentStatusReceived(AgentStatusEvent status)
    {
        DispatcherQueue.TryEnqueue(() =>
        {
            UpdateStatusBar("info", LocalizeStage(status.Stage), LocalizeStatusDetail(status.Detail));
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

    private void AppendMessage(string author, string text)
    {
        var isUser = author == "你";
        var isAssistant = author == "助手";
        var bubbleText = isUser || isAssistant ? text : $"{author}: {text}";
        var bubbleContent = CreateMessageContent(bubbleText, isAssistant, isUser);

        var bubble = new Border
        {
            MaxWidth = 680,
            Padding = new Thickness(12, 9, 12, 9),
            CornerRadius = new CornerRadius(8),
            Background = isUser
                ? (Brush)Application.Current.Resources["AccentFillColorDefaultBrush"]
                : (Brush)Application.Current.Resources["CardBackgroundFillColorSecondaryBrush"],
            BorderBrush = isUser
                ? null
                : (Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"],
            BorderThickness = isUser ? new Thickness(0) : new Thickness(1),
            HorizontalAlignment = isUser ? HorizontalAlignment.Right : HorizontalAlignment.Left,
            Child = bubbleContent
        };

        var label = new TextBlock
        {
            Text = author,
            Opacity = 0.72,
            HorizontalAlignment = bubble.HorizontalAlignment
        };

        var stack = new StackPanel
        {
            Spacing = 4,
            HorizontalAlignment = isUser ? HorizontalAlignment.Right : HorizontalAlignment.Left
        };
        stack.Children.Add(label);
        stack.Children.Add(bubble);

        if (!isUser && !isAssistant)
        {
            stack.Children.Remove(label);
        }

        MessagesPanel.Children.Add(stack);
    }

    private void ShowThinkingIndicator()
    {
        RemoveThinkingIndicator();

        _thinkingDotStep = 0;

        var dotBrush = (Brush)Application.Current.Resources["TextFillColorPrimaryBrush"];
        var dots = new TextBlock[3];
        for (var i = 0; i < 3; i++)
        {
            dots[i] = new TextBlock
            {
                Text = "●",
                Opacity = 0.25,
                FontSize = 12,
                Foreground = dotBrush,
                VerticalAlignment = VerticalAlignment.Center,
            };
        }

        var dotsPanel = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 6,
            Padding = new Thickness(2, 2, 2, 2),
        };
        foreach (var dot in dots)
        {
            dotsPanel.Children.Add(dot);
        }

        var bubble = new Border
        {
            MaxWidth = 680,
            Padding = new Thickness(14, 10, 14, 10),
            CornerRadius = new CornerRadius(8),
            Background = (Brush)Application.Current.Resources["CardBackgroundFillColorSecondaryBrush"],
            BorderBrush = (Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"],
            BorderThickness = new Thickness(1),
            HorizontalAlignment = HorizontalAlignment.Left,
            Child = dotsPanel,
        };

        var label = new TextBlock
        {
            Text = "助手",
            Opacity = 0.72,
            HorizontalAlignment = HorizontalAlignment.Left,
        };

        var stack = new StackPanel
        {
            Spacing = 4,
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        stack.Children.Add(label);
        stack.Children.Add(bubble);

        _thinkingIndicator = stack;
        MessagesPanel.Children.Add(stack);

        _thinkingDotsTimer = new DispatcherTimer
        {
            Interval = TimeSpan.FromMilliseconds(350),
        };
        _thinkingDotsTimer.Tick += (_, _) =>
        {
            _thinkingDotStep = (_thinkingDotStep + 1) % 4;
            for (var i = 0; i < 3; i++)
            {
                dots[i].Opacity = i < _thinkingDotStep ? 1.0 : 0.25;
            }
        };
        _thinkingDotsTimer.Start();
    }

    private void RemoveThinkingIndicator()
    {
        _thinkingDotsTimer?.Stop();
        _thinkingDotsTimer = null;
        _thinkingDotStep = 0;
        if (_thinkingIndicator is not null)
        {
            MessagesPanel.Children.Remove(_thinkingIndicator);
            _thinkingIndicator = null;
        }
    }

    private FrameworkElement CreateMessageContent(string text, bool isAssistant, bool isUser)
    {
        if (isAssistant && TryExtractMarkdownPayload(text, out var markdown))
        {
            return CreateMarkdownContent(markdown);
        }

        return new TextBlock
        {
            Text = text,
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = true,
            Foreground = isUser
                ? (Brush)Application.Current.Resources["TextOnAccentFillColorPrimaryBrush"]
                : (Brush)Application.Current.Resources["TextFillColorPrimaryBrush"]
        };
    }

    private FrameworkElement CreateMarkdownContent(string markdown)
    {
        var stack = new StackPanel
        {
            Spacing = 10
        };

        var copyButton = new Button
        {
            Content = "复制 Markdown",
            Padding = new Thickness(8, 4, 8, 4),
            HorizontalAlignment = HorizontalAlignment.Right,
            MinWidth = 0
        };
        copyButton.Click += (_, _) => CopyTextToClipboard(markdown);
        stack.Children.Add(copyButton);

        foreach (var block in ParseMarkdownBlocks(markdown))
        {
            if (block.IsCode)
            {
                stack.Children.Add(CreateCodeBlock(block.Text, block.Language));
                continue;
            }

            foreach (var element in CreateMarkdownTextElements(block.Text))
            {
                stack.Children.Add(element);
            }
        }

        return stack;
    }

    private IEnumerable<FrameworkElement> CreateMarkdownTextElements(string text)
    {
        var lines = text.Replace("\r\n", "\n").Split('\n');
        foreach (var rawLine in lines)
        {
            var line = rawLine.TrimEnd();
            if (string.IsNullOrWhiteSpace(line))
            {
                yield return new Border { Height = 4, Opacity = 0 };
                continue;
            }

            var textBlock = new TextBlock
            {
                TextWrapping = TextWrapping.Wrap,
                IsTextSelectionEnabled = true,
                Foreground = (Brush)Application.Current.Resources["TextFillColorPrimaryBrush"]
            };

            if (line.StartsWith("# "))
            {
                ApplyInlineMarkdown(textBlock, line[2..].Trim());
                textBlock.FontSize = 20;
                textBlock.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            }
            else if (line.StartsWith("## "))
            {
                ApplyInlineMarkdown(textBlock, line[3..].Trim());
                textBlock.FontSize = 18;
                textBlock.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            }
            else if (line.StartsWith("### "))
            {
                ApplyInlineMarkdown(textBlock, line[4..].Trim());
                textBlock.FontSize = 16;
                textBlock.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            }
            else if (line.StartsWith("- ") || line.StartsWith("* "))
            {
                textBlock.Inlines.Add(new Run { Text = "• " });
                AppendInlineMarkdown(textBlock.Inlines, line[2..].Trim());
            }
            else if (char.IsDigit(line[0]) && line.Contains(". "))
            {
                var dotIndex = line.IndexOf(". ", StringComparison.Ordinal);
                if (dotIndex > 0 && line.Take(dotIndex).All(char.IsDigit))
                {
                    textBlock.Inlines.Add(new Run { Text = line[..(dotIndex + 2)] });
                    AppendInlineMarkdown(textBlock.Inlines, line[(dotIndex + 2)..].Trim());
                }
                else
                {
                    ApplyInlineMarkdown(textBlock, line);
                }
            }
            else if (line.StartsWith("> ") || line.StartsWith(">"))
            {
                // Blockquote: left border + muted italic text
                var quoteText = line.StartsWith("> ") ? line[2..] : line[1..];
                textBlock.FontStyle = Windows.UI.Text.FontStyle.Italic;
                textBlock.Foreground = (Brush)Application.Current.Resources["TextFillColorSecondaryBrush"];
                textBlock.Padding = new Thickness(12, 4, 4, 4);
                ApplyInlineMarkdown(textBlock, quoteText.Trim());

                var border = new Border
                {
                    BorderBrush = (Brush)Application.Current.Resources["ControlStrokeColorDefaultBrush"],
                    BorderThickness = new Thickness(3, 0, 0, 0),
                    Child = textBlock,
                    Margin = new Thickness(0, 2, 0, 2)
                };
                yield return border;
                continue;
            }
            else
            {
                ApplyInlineMarkdown(textBlock, line);
            }

            yield return textBlock;
        }
    }

    private static void ApplyInlineMarkdown(TextBlock textBlock, string text)
    {
        textBlock.Inlines.Clear();
        AppendInlineMarkdown(textBlock.Inlines, text);
    }

    private static void AppendInlineMarkdown(InlineCollection inlines, string text)
    {
        if (string.IsNullOrEmpty(text))
        {
            return;
        }

        var index = 0;
        while (index < text.Length)
        {
            if (index + 1 < text.Length
                && text[index] == '*'
                && text[index + 1] == '*')
            {
                var closeIndex = text.IndexOf("**", index + 2, StringComparison.Ordinal);
                if (closeIndex > index + 1)
                {
                    var span = new Span
                    {
                        FontWeight = Microsoft.UI.Text.FontWeights.SemiBold
                    };
                    AppendInlineMarkdown(span.Inlines, text[(index + 2)..closeIndex]);
                    inlines.Add(span);
                    index = closeIndex + 2;
                    continue;
                }
            }

            if (text[index] == '`')
            {
                var closeIndex = text.IndexOf('`', index + 1);
                if (closeIndex > index)
                {
                    var span = new Span
                    {
                        FontFamily = new FontFamily("Consolas"),
                        Background = GetThemeBrush("CodeInlineBackgroundBrush"),
                        Foreground = GetThemeBrush("CodeInlineForegroundBrush")
                    };
                    span.Inlines.Add(new Run { Text = text[(index + 1)..closeIndex] });
                    inlines.Add(span);
                    index = closeIndex + 1;
                    continue;
                }
            }

            if (text[index] == '*')
            {
                var closeIndex = text.IndexOf('*', index + 1);
                if (closeIndex > index + 1)
                {
                    var span = new Span
                    {
                        FontStyle = Windows.UI.Text.FontStyle.Italic
                    };
                    AppendInlineMarkdown(span.Inlines, text[(index + 1)..closeIndex]);
                    inlines.Add(span);
                    index = closeIndex + 1;
                    continue;
                }
            }

            var nextIndex = FindNextInlineMarker(text, index);
            inlines.Add(new Run
            {
                Text = text[index..nextIndex]
            });
            index = nextIndex;
        }
    }

    private static int FindNextInlineMarker(string text, int startIndex)
    {
        var nextIndex = text.Length;
        foreach (var marker in new[] { "**", "*", "`" })
        {
            var index = text.IndexOf(marker, startIndex, StringComparison.Ordinal);
            if (index >= 0 && index < nextIndex)
            {
                nextIndex = index;
            }
        }

        return nextIndex;
    }

    private FrameworkElement CreateCodeBlock(string code, string? language)
    {
        var header = new Grid();
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var label = new TextBlock
        {
            Text = string.IsNullOrWhiteSpace(language) ? "code" : language.Trim(),
            Opacity = 0.72,
            VerticalAlignment = VerticalAlignment.Center
        };

        var copyButton = new Button
        {
            Content = "复制代码",
            Padding = new Thickness(8, 4, 8, 4),
            MinWidth = 0,
            HorizontalAlignment = HorizontalAlignment.Right
        };
        copyButton.Click += (_, _) => CopyTextToClipboard(code);

        Grid.SetColumn(label, 0);
        Grid.SetColumn(copyButton, 1);
        header.Children.Add(label);
        header.Children.Add(copyButton);

        var codeText = new TextBlock
        {
            Text = code,
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = true,
            FontFamily = new FontFamily("Consolas"),
            Foreground = GetThemeBrush("CodeBlockForegroundBrush")
        };

        return new Border
        {
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(10),
            Background = GetThemeBrush("CodeBlockBackgroundBrush"),
            Child = new StackPanel
            {
                Spacing = 8,
                Children =
                {
                    header,
                    codeText
                }
            }
        };
    }

    private IEnumerable<(bool IsCode, string Text, string? Language)> ParseMarkdownBlocks(string markdown)
    {
        var normalized = markdown.Replace("\r\n", "\n");
        var parts = normalized.Split("```");
        for (var i = 0; i < parts.Length; i++)
        {
            if (i % 2 == 0)
            {
                if (!string.IsNullOrWhiteSpace(parts[i]))
                {
                    yield return (false, parts[i].Trim(), null);
                }

                continue;
            }

            var block = parts[i];
            var firstNewline = block.IndexOf('\n');
            if (firstNewline < 0)
            {
                yield return (true, block.Trim(), null);
                continue;
            }

            var language = block[..firstNewline].Trim();
            var code = block[(firstNewline + 1)..].TrimEnd();
            yield return (true, code, string.IsNullOrWhiteSpace(language) ? null : language);
        }
    }

    private static bool TryExtractMarkdownPayload(string text, out string markdown)
    {
        var trimmed = text.Trim();
        if (trimmed.StartsWith(MarkdownOpenTag, StringComparison.OrdinalIgnoreCase)
            && trimmed.EndsWith(MarkdownCloseTag, StringComparison.OrdinalIgnoreCase))
        {
            markdown = trimmed[MarkdownOpenTag.Length..^MarkdownCloseTag.Length].Trim();
            return true;
        }

        if (LooksLikeMarkdownPayload(trimmed))
        {
            markdown = trimmed;
            return true;
        }

        markdown = string.Empty;
        return false;
    }

    private static bool LooksLikeMarkdownPayload(string text)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return false;
        }

        if (text.Contains("```", StringComparison.Ordinal))
        {
            return true;
        }

        if (text.Contains("**", StringComparison.Ordinal)
            || text.Contains('`')
            || HasStandaloneItalicMarker(text))
        {
            return true;
        }

        var lines = text.Replace("\r\n", "\n").Split('\n');
        var bulletLines = 0;
        var numberedLines = 0;
        var headingLines = 0;
        var blockquoteLines = 0;

        foreach (var rawLine in lines)
        {
            var line = rawLine.Trim();
            if (string.IsNullOrWhiteSpace(line))
            {
                continue;
            }

            if (line.StartsWith("# "))
            {
                headingLines++;
                continue;
            }

            if (line.StartsWith("## ") || line.StartsWith("### "))
            {
                headingLines++;
                continue;
            }

            if (line.StartsWith("- ") || line.StartsWith("* "))
            {
                bulletLines++;
                continue;
            }

            if (line.StartsWith("> ") || (line.Length > 1 && line.StartsWith(">")))
            {
                blockquoteLines++;
                continue;
            }

            var dotIndex = line.IndexOf(". ", StringComparison.Ordinal);
            if (dotIndex > 0 && line.Take(dotIndex).All(char.IsDigit))
            {
                numberedLines++;
            }
        }

        if (headingLines > 0)
        {
            return true;
        }

        if (blockquoteLines > 0)
        {
            return true;
        }

        if (bulletLines >= 2 || numberedLines >= 2)
        {
            return true;
        }

        if ((bulletLines + numberedLines) >= 1 && lines.Length >= 4)
        {
            return true;
        }

        return false;
    }

    private static bool HasStandaloneItalicMarker(string text)
    {
        var first = text.IndexOf('*');
        if (first < 0 || first + 1 >= text.Length)
        {
            return false;
        }

        var second = text.IndexOf('*', first + 1);
        return second > first + 1;
    }

    private void CopyTextToClipboard(string text)
    {
        var package = new DataPackage();
        package.SetText(text);
        Clipboard.SetContent(package);
        Clipboard.Flush();
        UpdateStatusBar("success", "已复制", "消息内容已复制到剪贴板。");
    }

    private void AppendAttachmentPreviews(IReadOnlyList<ChatAttachment> attachments, string role)
    {
        if (attachments.Count == 0)
        {
            return;
        }

        var wrap = new WrapPanel
        {
            Orientation = Orientation.Horizontal,
            ItemWidth = 142,
            ItemHeight = 178,
            HorizontalAlignment = role == "user" ? HorizontalAlignment.Right : HorizontalAlignment.Left,
            Margin = new Thickness(0, 2, 0, 0)
        };

        foreach (var attachment in attachments)
        {
            wrap.Children.Add(CreateChatAttachmentPreview(attachment, removable: false));
        }

        MessagesPanel.Children.Add(wrap);
    }

    private void RefreshAttachments()
    {
        AttachmentPanel.Children.Clear();
        AttachmentScroller.Visibility = _attachments.Count == 0 ? Visibility.Collapsed : Visibility.Visible;

        foreach (var attachment in _attachments)
        {
            AttachmentPanel.Children.Add(CreateAttachmentChip(attachment));
        }
    }

    private void AddImageAttachments(IEnumerable<StorageFile> files)
    {
        var added = 0;
        foreach (var file in files)
        {
            if (_attachments.Any(item => item.Path == file.Path))
            {
                continue;
            }

            _attachments.Add(new ChatAttachment(file.Path, file.Name));
            added++;
        }

        if (added == 0)
        {
            UpdateStatusBar("info", "图片已存在", $"当前已附加 {_attachments.Count} 张图片。");
            return;
        }

        RefreshAttachments();
        UpdateStatusBar("success", "图片已添加", $"本次添加 {added} 张，当前共 {_attachments.Count} 张图片。");
    }

    private static bool IsSupportedImageFile(StorageFile file)
    {
        var extension = Path.GetExtension(file.Name);
        return IsSupportedImageExtension(extension);
    }

    private static bool IsSupportedImagePath(string path)
    {
        return IsSupportedImageExtension(Path.GetExtension(path));
    }

    private static bool IsSupportedImageExtension(string? extension)
    {
        extension ??= string.Empty;
        return extension.Equals(".png", StringComparison.OrdinalIgnoreCase)
            || extension.Equals(".jpg", StringComparison.OrdinalIgnoreCase)
            || extension.Equals(".jpeg", StringComparison.OrdinalIgnoreCase)
            || extension.Equals(".webp", StringComparison.OrdinalIgnoreCase)
            || extension.Equals(".bmp", StringComparison.OrdinalIgnoreCase)
            || extension.Equals(".gif", StringComparison.OrdinalIgnoreCase);
    }

    private FrameworkElement CreateAttachmentChip(ChatAttachment attachment)
    {
        var dot = new Border
        {
            Width = 10,
            Height = 10,
            CornerRadius = new CornerRadius(999),
            Background = GetThemeBrush("AttachmentDotBrush"),
            BorderBrush = GetThemeBrush("AttachmentBorderBrush"),
            BorderThickness = new Thickness(1),
            Margin = new Thickness(0, 0, 2, 0)
        };

        ToolTipService.SetToolTip(dot, $"{attachment.Name}\n单击预览，右键移除");
        dot.Tapped += async (_, _) => await ShowImagePreviewDialogAsync(attachment, removable: true);
        dot.RightTapped += (_, _) =>
        {
            _attachments.RemoveAll(item => item.Path == attachment.Path);
            RefreshAttachments();
            UpdateStatusBar("info", "图片已移除", $"当前还剩 {_attachments.Count} 张图片。");
        };

        return dot;
    }

    private FrameworkElement CreateChatAttachmentPreview(ChatAttachment attachment, bool removable)
    {
        var image = new Image
        {
            Width = 120,
            Height = 120,
            Stretch = Stretch.UniformToFill,
            Opacity = 0.2
        };
        image.Tapped += async (_, _) => await ShowImagePreviewDialogAsync(attachment);

        var stack = new StackPanel
        {
            Spacing = 6
        };
        stack.Children.Add(image);

        if (removable)
        {
            var removeButton = new Button
            {
                Content = "移除",
                Padding = new Thickness(8, 4, 8, 4),
                HorizontalAlignment = HorizontalAlignment.Stretch
            };
            removeButton.Click += (_, _) =>
            {
                _attachments.RemoveAll(item => item.Path == attachment.Path);
                RefreshAttachments();
            };
            stack.Children.Add(removeButton);
        }

        var preview = new Border
        {
            Width = 132,
            Padding = new Thickness(6),
            CornerRadius = new CornerRadius(8),
            Background = (Brush)Application.Current.Resources["CardBackgroundFillColorSecondaryBrush"],
            BorderBrush = (Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"],
            BorderThickness = new Thickness(1),
            Child = stack
        };
        _ = LoadImagePreviewAsync(image, attachment.Path);
        return preview;
    }

    private async Task LoadImagePreviewAsync(Image image, string path)
    {
        try
        {
            var bitmap = await LoadPreviewBitmapAsync(path);
            if (bitmap is null)
            {
                return;
            }

            image.Source = bitmap;
            image.Opacity = 1;
        }
        catch
        {
            image.Opacity = 0.35;
        }
    }

    private async Task ShowImagePreviewDialogAsync(ChatAttachment attachment, bool removable = false)
    {
        var image = new Image
        {
            MaxWidth = 960,
            MaxHeight = 680,
            Stretch = Stretch.Uniform
        };

        try
        {
            image.Source = await LoadPreviewBitmapAsync(attachment.Path);
        }
        catch
        {
            image.Opacity = 0.35;
        }

        var dialog = new ContentDialog
        {
            XamlRoot = RootGrid.XamlRoot,
            Title = "图片预览",
            Content = new ScrollViewer
            {
                HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
                Content = image
            },
            CloseButtonText = "关闭",
            SecondaryButtonText = removable ? "移除" : string.Empty
        };
        var result = await dialog.ShowAsync();
        if (removable && result == ContentDialogResult.Secondary)
        {
            _attachments.RemoveAll(item => item.Path == attachment.Path);
            RefreshAttachments();
            UpdateStatusBar("info", "图片已移除", $"当前还剩 {_attachments.Count} 张图片。");
        }
    }

    private async Task<BitmapImage?> LoadPreviewBitmapAsync(string path)
    {
        var dataUrl = await _backendClient.SendAsync<string>("readImagePreview", new { path });
        if (string.IsNullOrWhiteSpace(dataUrl))
        {
            return null;
        }

        var bytes = DecodeDataUrl(dataUrl);
        using var stream = new InMemoryRandomAccessStream();
        await stream.WriteAsync(bytes.AsBuffer());
        stream.Seek(0);

        var bitmap = new BitmapImage();
        await bitmap.SetSourceAsync(stream);
        return bitmap;
    }

    private static byte[] DecodeDataUrl(string dataUrl)
    {
        var commaIndex = dataUrl.IndexOf(',');
        var base64 = commaIndex >= 0 ? dataUrl[(commaIndex + 1)..] : dataUrl;
        return Convert.FromBase64String(base64);
    }

    private static string ShortenPath(string path)
    {
        if (string.IsNullOrWhiteSpace(path))
        {
            return "图片";
        }

        var fileName = Path.GetFileName(path);
        var directoryName = Path.GetFileName(Path.GetDirectoryName(path) ?? string.Empty);
        var label = string.IsNullOrWhiteSpace(directoryName)
            ? fileName
            : $"{directoryName}\\{fileName}";
        return label.Length <= 34 ? label : $"...{label[^31..]}";
    }

    private async Task<bool> TryHandleClipboardImagePasteAsync()
    {
        try
        {
            var dataView = Clipboard.GetContent();
            if (dataView is null)
            {
                return false;
            }

            if (dataView.Contains(StandardDataFormats.StorageItems))
            {
                var items = await dataView.GetStorageItemsAsync();
                var files = items
                    .OfType<StorageFile>()
                    .Where(IsSupportedImageFile)
                    .ToArray();
                if (files.Length > 0)
                {
                    AddImageAttachments(files);
                    return true;
                }
            }

            if (!dataView.Contains(StandardDataFormats.Bitmap))
            {
                return false;
            }

            var bitmapReference = await dataView.GetBitmapAsync();
            if (bitmapReference is null)
            {
                return false;
            }

            var file = await SaveClipboardBitmapAsync(bitmapReference);
            AddImageAttachments(new[] { file });
            return true;
        }
        catch (Exception error)
        {
            ShowError("粘贴图片失败", error);
            return false;
        }
    }

    private static async Task<StorageFile> SaveClipboardBitmapAsync(RandomAccessStreamReference bitmapReference)
    {
        Directory.CreateDirectory(ClipboardAttachmentDirectory);

        using var sourceStream = await bitmapReference.OpenReadAsync();
        using var memoryStream = sourceStream.AsStreamForRead();
        using var buffer = new MemoryStream();
        await memoryStream.CopyToAsync(buffer);

        var fileName = $"clipboard-{DateTimeOffset.Now:yyyyMMdd-HHmmssfff}.png";
        var filePath = Path.Combine(ClipboardAttachmentDirectory, fileName);
        await File.WriteAllBytesAsync(filePath, buffer.ToArray());
        var file = await StorageFile.GetFileFromPathAsync(filePath);
        PruneClipboardImages();
        return file;
    }

    private static void PruneClipboardImages()
    {
        try
        {
            if (!Directory.Exists(ClipboardAttachmentDirectory))
            {
                return;
            }

            var files = new DirectoryInfo(ClipboardAttachmentDirectory)
                .GetFiles("clipboard-*.png")
                .OrderByDescending(f => f.CreationTimeUtc)
                .ToArray();

            for (var i = MaxClipboardImages; i < files.Length; i++)
            {
                try
                {
                    files[i].Delete();
                }
                catch (IOException)
                {
                    // File may be in use; skip
                }
            }
        }
        catch (Exception)
        {
            // Best-effort cleanup; don't disrupt user
        }
    }

    private void EnsureWindowFileDropHook(nint hwnd)
    {
        if (hwnd == 0 || _windowProcDelegate is not null)
        {
            return;
        }

        _windowProcDelegate = WindowProc;
        var newWindowProc = Marshal.GetFunctionPointerForDelegate(_windowProcDelegate);
        _originalWindowProc = SetWindowLongPtr(hwnd, WindowLongPtrWndProc, newWindowProc);
        DragAcceptFiles(hwnd, true);
    }

    #region Auto-wake timer

    private DateTime? _lastAutoWakeTime;
    private bool _autoWakeInProgress;
    private bool _isStopping;

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
            "gpt-4o-mini",
            "gpt-4o",
            "gpt-4.1-mini",
            "gpt-4.1",
            "o3-mini",
            "o4-mini",
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
        if (_autoWakeInProgress) return;
        if (!IsInAutoWakeWindow()) return;

        var interval = TimeSpan.FromMinutes(Math.Max(1, (int)(_settings?.AutoWakeIntervalMinutes ?? 30)));
        var now = DateTime.Now;
        if (_lastAutoWakeTime.HasValue && (now - _lastAutoWakeTime.Value) < interval) return;

        _autoWakeInProgress = true;
        try
        {
            await _backendClient.EnsureConnectedAsync();
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(10));
            await _backendClient.SendAsync("ping", new { }, cts.Token);
            _lastAutoWakeTime = DateTime.Now;
            LogStartup("自动唤醒完成: ping ok");
        }
        catch (Exception error)
        {
            LogStartup($"自动唤醒失败: {LocalizeError(error.Message)}");
        }
        finally
        {
            _autoWakeInProgress = false;
            ShowNextWakeTime();
        }
    }

    private DateTime? GetNextAutoWakeTime()
    {
        var settings = _settings;
        if (settings == null || !settings.AutoWakeEnabled) return null;

        var intervalMinutes = Math.Max(1, (int)settings.AutoWakeIntervalMinutes);
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
            for (int i = 0; i < 200; i++)
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

    private void ShowError(string title, Exception error, bool addMessage = true)
    {
        UpdateStatusBar("error", title, LocalizeError(error.Message));
        if (addMessage)
        {
            AppendMessage("错误", LocalizeError(error.Message));
        }
    }

    private async Task ShowStartupFailureAsync(Exception error, string stderrTail)
    {
        var detail = LocalizeError(error.Message);
        if (!string.IsNullOrWhiteSpace(stderrTail))
        {
            detail = $"{detail}\n\n后端日志:\n{stderrTail}";
        }

        var dialog = new ContentDialog
        {
            XamlRoot = RootGrid.XamlRoot,
            Title = "启动失败",
            Content = $"无法连接本地后端：{detail}",
            CloseButtonText = "关闭"
        };
        await dialog.ShowAsync();
    }

    private async Task<T?> SendWithTimeoutAsync<T>(
        Func<CancellationToken, Task<T?>> action,
        string step,
        int timeoutMs = 8000)
    {
        using var cts = new CancellationTokenSource(timeoutMs);
        try
        {
            return await action(cts.Token).WaitAsync(cts.Token);
        }
        catch (TimeoutException)
        {
            throw new InvalidOperationException($"启动超时：{step}");
        }
        catch (OperationCanceledException)
        {
            throw new InvalidOperationException($"启动超时：{step}");
        }
    }

    private void UpdateStartupStep(string step)
    {
        _startupStep = step;
        UpdateStatusBar("info", "正在启动", $"{step}...");
        LogStartup($"Step: {step}");
    }

    private void UpdateStatusBar(string level, string title, string message)
    {
        StatusBarTitle.Text = title;
        StatusBarMessage.Text = message;
        StatusBarIcon.Foreground = level switch
        {
            "error" => BrushRed,
            "warning" => BrushOrange,
            "success" => BrushGreen,
            _ => (Brush)Application.Current.Resources["TextFillColorSecondaryBrush"]
        };
        StatusBarIcon.Glyph = level switch
        {
            "error" => "\uE783",
            "warning" => "\uE7BA",
            "success" => "\uE73E",
            _ => "\uE946"
        };
    }

    private void RestoreIdleStatus(string title = "就绪", string message = "已收到回复")
    {
        if (_updateDownloadPercent >= 0)
        {
            UpdateStatusBar("info", "正在下载更新", $"正在下载 {_updateDownloadVersion}... {_updateDownloadPercent}%");
        }
        else
        {
            UpdateStatusBar("success", title, message);
        }
    }

    private static string StartupLogPath()
    {
        var root = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "com.local.vaultpilot");
        Directory.CreateDirectory(root);
        return Path.Combine(root, "startup.log");
    }

    private static void LogStartup(string message)
    {
        try
        {
            var line = $"{DateTimeOffset.Now:O} {message}";
            File.AppendAllText(StartupLogPath(), line + Environment.NewLine, Encoding.UTF8);
        }
        catch
        {
            // Ignore logging failures.
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

    private void RenderCurrentSession()
    {
        MessagesPanel.Children.Clear();
        var session = CurrentSession();
        if (session is null || session.Turns.Count == 0)
        {
            RefreshContextStatus();
            return;
        }

        foreach (var turn in session.Turns)
        {
            AppendMessage(turn.Role == "user" ? "你" : "助手", turn.Text);
            if (turn.Attachments is { Count: > 0 })
            {
                AppendAttachmentPreviews(turn.Attachments, turn.Role);
            }

            if (turn.Role == "assistant")
            {
                if (turn.ThinkingTrace is { Steps.Count: > 0 } trace)
                {
                    AppendThinkingTrace(trace);
                }

                if (turn.Citations is { Count: > 0 } citations)
                {
                    AppendCitationCards(citations);
                }

                if (turn.SavedNote is not null)
                {
                    AppendMessage("系统", $"已保存笔记：{turn.SavedNote.Title}");
                }
            }
        }
        RefreshContextStatus();
    }

    private void AppendThinkingTrace(ThinkingTrace trace)
    {
        var stepsPanel = new StackPanel { Spacing = 4 };
        foreach (var step in trace.Steps)
        {
            var stepBlock = new TextBlock
            {
                Text = $"• {step.Title}: {step.Detail}",
                FontSize = 12,
                Opacity = 0.7,
                TextWrapping = TextWrapping.Wrap
            };
            stepsPanel.Children.Add(stepBlock);
        }

        var expander = new Expander
        {
            Header = $"💭 思考过程 ({trace.Steps.Count} 步){(string.IsNullOrWhiteSpace(trace.Summary) ? "" : $" — {trace.Summary}")}",
            IsExpanded = false,
            HorizontalAlignment = HorizontalAlignment.Left,
            MaxWidth = 680,
            Content = stepsPanel
        };

        MessagesPanel.Children.Add(expander);
    }

    private void AppendCitationCards(IReadOnlyList<AnswerCitation> citations)
    {
        var citationsPanel = new StackPanel
        {
            Spacing = 4,
            HorizontalAlignment = HorizontalAlignment.Left,
            MaxWidth = 680,
            Margin = new Thickness(0, 4, 0, 0)
        };

        var header = new TextBlock
        {
            Text = $"📚 引用 ({citations.Count})",
            FontSize = 12,
            Opacity = 0.7,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold
        };
        citationsPanel.Children.Add(header);

        foreach (var citation in citations)
        {
            var card = new Border
            {
                Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"],
                BorderBrush = (Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"],
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(4),
                Padding = new Thickness(8, 4, 8, 4),
                Child = new StackPanel
                {
                    Spacing = 2,
                    Children =
                    {
                        new TextBlock
                        {
                            Text = citation.Title,
                            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
                            FontSize = 12
                        },
                        new TextBlock
                        {
                            Text = citation.Snippet,
                            FontSize = 11,
                            Opacity = 0.8,
                            TextWrapping = TextWrapping.Wrap,
                            MaxLines = 3,
                            TextTrimming = TextTrimming.CharacterEllipsis
                        }
                    }
                }
            };
            citationsPanel.Children.Add(card);
        }

        MessagesPanel.Children.Add(citationsPanel);
    }

    private ChatSession? CurrentSession()
    {
        return _chatState.Sessions.FirstOrDefault(session => session.Id == _currentSessionId)
            ?? _chatState.Sessions.FirstOrDefault();
    }

    private ConversationTurn[] GetConversationHistory()
    {
        var session = CurrentSession();
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

    private async Task CompressCurrentSessionIfNeededAsync(
        string pendingText,
        IReadOnlyList<ChatAttachment> pendingAttachments)
    {
        var session = CurrentSession();
        if (session is null)
        {
            return;
        }

        var contextWindow = ResolveContextWindowTokens();
        var projectedTokens = EstimateSessionTokens(session) + EstimateTurnTokens(pendingText, pendingAttachments);
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

        var summary = await _backendClient.SendAsync<ConversationSummary>(
            "compressChatHistory",
            new
            {
                summary = session.Summary,
                history = compressibleTurns
            });
        if (summary is null)
        {
            return;
        }

        var now = DateTimeOffset.UtcNow.ToString("O");
        var updated = session with
        {
            Summary = summary,
            Turns = session.Turns.Skip(compressibleCount).ToArray(),
            UpdatedAt = now
        };
        await _chatStateLock.WaitAsync();
        try
        {
            var sessions = _chatState.Sessions
                .Select(item => item.Id == updated.Id ? updated : item)
                .ToArray();
            _chatState = new ChatState(updated.Id, sessions);
            _currentSessionId = updated.Id;
        }
        finally
        {
            _chatStateLock.Release();
        }
        await SaveChatStateAsync();
        RefreshSessions();
        RenderCurrentSession();
    }

    private void RefreshContextStatus()
    {
        var session = CurrentSession();
        var contextWindow = ResolveContextWindowTokens();
        var usedTokens = session is null ? 0 : EstimateSessionTokens(session);
        var remainingTokens = usedTokens >= contextWindow ? 0 : contextWindow - usedTokens;
        var remainingPercent = contextWindow == 0
            ? 100.0
            : Math.Clamp((double)remainingTokens / contextWindow * 100.0, 0.0, 100.0);
        var usedPercent = Math.Clamp(100.0 - remainingPercent, 0.0, 100.0);
        var usageBrush = remainingPercent switch
        {
            > 50 => BrushLimeGreen,
            > 20 => BrushOrange,
            _ => BrushRed
        };
        _contextUsagePercent = usedPercent;

        ContextUsageFill.Background = usageBrush;
        UpdateContextUsageBarVisual();

        var tooltip = $"上下文已用：{usedPercent:0.#}%；剩余：{remainingPercent:0.#}%（约 {FormatTokenCount(usedTokens)} / {FormatTokenCount(contextWindow)}）";
        ToolTipService.SetToolTip(ContextUsageBarHost, tooltip);
        ToolTipService.SetToolTip(ContextUsageTrack, tooltip);
        ToolTipService.SetToolTip(ContextUsageFill, tooltip);
    }

    private void OnContextUsageBarHostSizeChanged(object sender, SizeChangedEventArgs e)
    {
        UpdateContextUsageBarVisual();
    }

    private void UpdateContextUsageBarVisual()
    {
        var width = ContextUsageBarHost.ActualWidth;
        if (width <= 0)
        {
            return;
        }

        ContextUsageFill.Width = width * (_contextUsagePercent / 100.0);
    }

    private ulong EstimateSessionTokens(ChatSession session)
    {
        var total = EstimateTokensForText(session.Summary?.Text);
        foreach (var turn in session.Turns)
        {
            total += EstimateTurnTokens(turn.Text, turn.Attachments ?? Array.Empty<ChatAttachment>());
        }
        return total;
    }

    private static ulong EstimateTurnTokens(string? text, IReadOnlyList<ChatAttachment> attachments)
    {
        return EstimateTokensForText(text) + (ulong)attachments.Count * ImageAttachmentTokenEstimate;
    }

    private static ulong EstimateTokensForText(string? text)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return 0;
        }

        ulong ascii = 0;
        ulong nonAscii = 0;
        foreach (var item in text)
        {
            if (char.IsWhiteSpace(item))
            {
                continue;
            }

            if (item <= 0x7f)
            {
                ascii++;
            }
            else
            {
                nonAscii++;
            }
        }

        return nonAscii + ((ascii + 3) / 4);
    }

    private ulong ResolveContextWindowTokens()
    {
        var configuredLimit = _settings?.Provider.ContextWindowTokens;
        if (configuredLimit.HasValue && configuredLimit.Value > 0)
        {
            return configuredLimit.Value;
        }

        var model = (_settings?.Provider.Model ?? string.Empty).Trim().ToLowerInvariant();
        if (ContainsModelToken(model, "glm-5.1"))
        {
            return 200_000;
        }
        if (ContainsModelToken(model, "claude"))
        {
            return ContainsModelToken(model, "1m") ? 1_000_000UL : 200_000;
        }
        if (ContainsModelToken(model, "gpt-4.1") || ContainsModelToken(model, "gpt-5"))
        {
            return 1_047_576;
        }
        if (ContainsModelToken(model, "gpt-4o"))
        {
            return 128_000;
        }
        if (IsOpenAiOSeriesModel(model))
        {
            return 200_000;
        }
        if (ContainsModelToken(model, "gemini"))
        {
            return 1_000_000;
        }

        return 128_000;
    }

    /// <summary>
    /// Checks if the model name contains the given token as a distinct segment
    /// (preceded by start, '-', '_', '.', '/', or ' ', or followed by the same).
    /// Prevents false positives like "co1l" matching "o1".
    /// </summary>
    private static bool ContainsModelToken(string model, string token)
    {
        var index = model.IndexOf(token, StringComparison.Ordinal);
        while (index >= 0)
        {
            var beforeOk = index == 0 || IsModelSeparator(model[index - 1]);
            var afterPos = index + token.Length;
            var afterOk = afterPos >= model.Length || IsModelSeparator(model[afterPos]);
            if (beforeOk && afterOk) return true;
            index = model.IndexOf(token, index + 1, StringComparison.Ordinal);
        }
        return false;
    }

    /// <summary>
    /// OpenAI o-series models: o1, o3, o4 (with optional suffix like -mini, -preview).
    /// Matches "o1", "o1-mini", "o3-mini", "o4-mini" etc. but not "co1l" or "po3".
    /// </summary>
    private static bool IsOpenAiOSeriesModel(string model)
    {
        // Check for o1/o3/o4 at word boundary followed by end, separator, or hyphen
        foreach (var prefix in new[] { "o1", "o3", "o4" })
        {
            var index = model.IndexOf(prefix, StringComparison.Ordinal);
            while (index >= 0)
            {
                var beforeOk = index == 0 || IsModelSeparator(model[index - 1]);
                var afterPos = index + prefix.Length;
                var afterOk = afterPos >= model.Length || IsModelSeparator(model[afterPos]);
                if (beforeOk && afterOk) return true;
                index = model.IndexOf(prefix, index + 1, StringComparison.Ordinal);
            }
        }
        return false;
    }

    private static bool IsModelSeparator(char c) =>
        c is '-' or '_' or '.' or '/' or ' ' or '(' or ')' or ':' or ',';

    private static string FormatTokenCount(ulong tokens)
    {
        if (tokens >= 1_000_000)
        {
            return $"{tokens / 1_000_000.0:0.#}M";
        }
        if (tokens >= 1_000)
        {
            return $"{tokens / 1_000.0:0.#}K";
        }
        return tokens.ToString();
    }

    private void AddTurn(
        string role,
        string text,
        GroundedAnswer? answer = null,
        IReadOnlyList<ChatAttachment>? attachments = null)
    {
        _chatStateLock.Wait();
        try
        {
            var session = CurrentSession();
            if (session is null)
            {
                EnsureCurrentSession();
                session = CurrentSession();
            }

            if (session is null)
            {
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
                now);

            var turns = new List<ChatTurn>(session.Turns.Count + 1);
            turns.AddRange(session.Turns);
            turns.Add(turn);
            var title = session.Title == "新对话" && role == "user"
                ? BuildSessionTitle(text)
                : session.Title;
            var updated = session with { Title = title, Turns = turns, UpdatedAt = now };
            var sessions = _chatState.Sessions
                .Select(item => item.Id == updated.Id ? updated : item)
                .OrderByDescending(item => item.UpdatedAt)
                .ToArray();

            _chatState = new ChatState(updated.Id, sessions);
            _currentSessionId = updated.Id;
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
            var saved = await _backendClient.SendAsync<ChatState>(
                "saveChatState",
                new { state = snapshot });

            if (saved is not null)
            {
                await _chatStateLock.WaitAsync();
                try
                {
                    _chatState = saved;
                }
                finally
                {
                    _chatStateLock.Release();
                }
            }
        }
        catch (Exception error)
        {
            UpdateStatusBar("warning", "聊天记录未保存", LocalizeError(error.Message));
        }
    }

    private void RefreshSessions()
    {
        SessionList.ItemsSource = _chatState.Sessions
            .Select(session => new SessionListItem(
                session.Id,
                session.Title,
                $"{session.Turns.Count} 条消息"))
            .ToList();
        SessionList.SelectedItem = SessionList.Items
            .OfType<SessionListItem>()
            .FirstOrDefault(item => item.Id == _currentSessionId);
        DeleteSessionButton.IsEnabled = _chatState.Sessions.Count > 0;
    }

    private static string BuildSessionTitle(string text)
    {
        var normalized = string.Join(" ", text.Split(Array.Empty<char>(), StringSplitOptions.RemoveEmptyEntries));
        return normalized.Length <= 28 ? normalized : $"{normalized[..28]}...";
    }

    private static string LocalizeStage(string stage)
    {
        return stage switch
        {
            "analyzing" => "正在分析",
            "compressing" => "正在压缩上下文",
            "responding" => "正在组织回复",
            "retrieving" => "正在检索",
            "ranking" => "正在排序",
            "executing" => "正在执行工具",
            "saving" => "正在保存",
            _ => stage
        };
    }

    private static string LocalizeStatusDetail(string detail)
    {
        return detail switch
        {
            "Analyzing request" => "正在分析请求",
            "Preparing request..." => "正在准备请求...",
            "Preparing answer" => "正在准备回复",
            "Preparing final answer" => "正在准备最终回复",
            "Loading recent notes" => "正在加载最近笔记",
            "No direct match; listing recent notes" => "没有直接命中，正在加载最近笔记",
            "Compressing earlier conversation context" => "正在压缩较早的对话内容",
            "Saving generated note" => "正在保存生成的笔记",
            _ when detail.StartsWith("Searching notes: ", StringComparison.Ordinal) =>
                $"正在搜索笔记：{detail["Searching notes: ".Length..]}",
            _ when detail.StartsWith("Ranking ", StringComparison.Ordinal) =>
                detail.Replace("Ranking ", "正在排序 ", StringComparison.Ordinal)
                    .Replace(" candidate notes", " 条候选笔记", StringComparison.Ordinal),
            _ when detail.StartsWith("Listing directory: ", StringComparison.Ordinal) =>
                $"正在列出目录：{detail["Listing directory: ".Length..]}",
            _ when detail.StartsWith("Reading file: ", StringComparison.Ordinal) =>
                $"正在读取文件：{detail["Reading file: ".Length..]}",
            _ when detail.StartsWith("Running command: ", StringComparison.Ordinal) =>
                $"正在执行命令：{detail["Running command: ".Length..]}",
            _ => detail
        };
    }

    private static string LocalizeError(string message)
    {
        return message
            .Replace("API key is empty", "API Key 为空，请先在设置中配置模型服务。", StringComparison.Ordinal)
            .Replace("The Rust backend process is not connected.", "Rust 后端尚未连接。", StringComparison.Ordinal)
            .Replace("The Rust backend process closed stdout.", "Rust 后端已关闭输出通道。", StringComparison.Ordinal)
            .Replace("Backend request failed.", "后端请求失败。", StringComparison.Ordinal)
            // Network errors
            .Replace("Connection refused", "连接被拒绝，后端服务可能未启动。", StringComparison.Ordinal)
            .Replace("Connection timed out", "连接超时，请检查网络或后端服务状态。", StringComparison.Ordinal)
            .Replace("A task was canceled.", "操作已取消。", StringComparison.Ordinal)
            .Replace("The operation was canceled.", "操作已取消。", StringComparison.Ordinal)
            // HTTP errors
            .Replace("401 Unauthorized", "认证失败（401），请检查 API Key 是否正确。", StringComparison.Ordinal)
            .Replace("403 Forbidden", "访问被拒绝（403），API Key 可能没有足够权限。", StringComparison.Ordinal)
            .Replace("429 Too Many Requests", "请求过于频繁（429），请稍后重试。", StringComparison.Ordinal)
            .Replace("500 Internal Server Error", "服务器内部错误（500），请稍后重试。", StringComparison.Ordinal)
            .Replace("502 Bad Gateway", "网关错误（502），服务可能正在重启。", StringComparison.Ordinal)
            .Replace("503 Service Unavailable", "服务不可用（503），请稍后重试。", StringComparison.Ordinal)
            // Model errors
            .Replace("model not found", "指定的模型不存在，请在设置中检查模型名称。", StringComparison.Ordinal)
            .Replace("Model not found", "指定的模型不存在，请在设置中检查模型名称。", StringComparison.Ordinal)
            .Replace("Invalid API key", "API Key 无效，请在设置中重新配置。", StringComparison.Ordinal)
            .Replace("insufficient_quota", "API 配额不足，请检查账户余额或提升套餐。", StringComparison.Ordinal)
            // File/IO errors
            .Replace("Access to the path", "文件访问被拒绝，可能正在被其他程序使用。", StringComparison.Ordinal)
            .Replace("The file is being used by another process", "文件正在被其他程序使用，请关闭后重试。", StringComparison.Ordinal)
            .Replace("No such file or directory", "文件或目录不存在。", StringComparison.Ordinal)
            .Replace("Directory not found", "目录不存在，请检查知识库路径设置。", StringComparison.Ordinal)
            // Generic fallback wrapping
            .Replace("An error occurred while sending the request.", "发送请求时发生错误，请检查网络连接。", StringComparison.Ordinal)
            .Replace("The SSL connection could not be established", "SSL 连接建立失败，请检查网络安全性设置。", StringComparison.Ordinal);
    }

    private sealed record SessionListItem(string Id, string Title, string Detail);
}
