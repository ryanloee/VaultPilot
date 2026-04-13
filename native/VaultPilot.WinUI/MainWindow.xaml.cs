using VaultPilot.WinUI.Backend;
using VaultPilot.WinUI.Controls;
using VaultPilot.WinUI.Models;
using Microsoft.UI.Input;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Text;
using Windows.Graphics;
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
    private readonly BackendClient _backendClient;
    private AppWindow? _appWindow;
    private ChatState _chatState = new(string.Empty, Array.Empty<ChatSession>());
    private string _currentSessionId = string.Empty;
    private AppSettings? _settings;
    private int _noteCount;
    private readonly List<ChatAttachment> _attachments = [];
    private bool _sidebarCollapsed = true;
    private bool _sidebarAutoCollapsed = true;
    private string _startupStep = "初始化";
    private volatile int _updateDownloadPercent = -1;
    private string _updateDownloadVersion = string.Empty;

    public MainWindow()
    {
        InitializeComponent();
        ConfigureWindowBounds();
        _backendClient = new BackendClient();
        _backendClient.AgentStatusReceived += OnAgentStatusReceived;
        RootGrid.Loaded += OnLoaded;
        Closed += OnClosed;
        SendButton.Click += OnSendClicked;
        RecordButton.Click += OnRecordClicked;
        SettingsButton.Click += OnSettingsClicked;
        RebuildButton.Click += OnRebuildClicked;
        ImportButton.Click += OnImportClicked;
        ComposerBox.KeyDown += OnComposerKeyDown;
        ComposerSendAccelerator.Invoked += OnComposerSendAcceleratorInvoked;
        SessionList.SelectionChanged += OnSessionSelectionChanged;
        DeleteSessionButton.Click += OnDeleteSessionClicked;
        NewSessionButton.Click += OnNewSessionClicked;
        AddImageButton.Click += OnAddImageClicked;
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
            var projectLinkButton = new Button
            {
                Content = "项目地址",
                HorizontalAlignment = HorizontalAlignment.Left
            };
            projectLinkButton.Click += async (_, _) => await OpenProjectHomepageAsync();
            var versionLabel = new TextBlock
            {
                Text = string.Empty,
                Opacity = 0.0,
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

            var result = await dialog.ShowAsync();
            if (result != ContentDialogResult.Primary)
            {
                return;
            }

            if (!ulong.TryParse(timeoutBox.Text.Trim(), out var timeoutMs) || timeoutMs == 0)
            {
                throw new InvalidOperationException("请求超时必须是大于 0 的数字。");
            }

            ulong? contextWindowTokens = null;
            if (!string.IsNullOrWhiteSpace(contextWindowBox.Text))
            {
                if (!ulong.TryParse(contextWindowBox.Text.Trim(), out var parsedContextWindow))
                {
                    throw new InvalidOperationException("上下文窗口 Token 数必须是数字。");
                }

                contextWindowTokens = parsedContextWindow;
            }

            var updated = new AppSettings(
                vaultBox.Text.Trim(),
                new ProviderConfig(
                    apiKeyBox.Password.Trim(),
                    baseUrlBox.Text.Trim(),
                    modelBox.Text.Trim(),
                    timeoutMs,
                    contextWindowTokens),
                autoCheckUpdatesBox.IsChecked ?? true);

            _settings = await _backendClient.SendAsync<AppSettings>("saveSettings", new { settings = updated });
            RefreshVaultSummary();
            RefreshContextStatus();
            UpdateStatusBar("success", "设置已保存", "模型服务配置已更新。");
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
        await SendCurrentMessageAsync();
    }

    private async void OnRecordClicked(object sender, RoutedEventArgs e)
    {
        await RecordCurrentMessageAsync();
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

        var remaining = _chatState.Sessions
            .Where(item => item.Id != session.Id)
            .ToArray();

        _chatState = new ChatState(
            remaining.FirstOrDefault()?.Id ?? string.Empty,
            remaining);
        _currentSessionId = _chatState.CurrentSessionId;
        EnsureCurrentSession();
        await SaveChatStateAsync();
        RefreshSessions();
        RenderCurrentSession();

        UpdateStatusBar("success", "会话已删除", $"已删除「{session.Title}」。");
    }

    private async void OnNewSessionClicked(object sender, RoutedEventArgs e)
    {
        var now = DateTimeOffset.UtcNow.ToString("O");
        var session = new ChatSession(
            Guid.NewGuid().ToString("N"),
            "新对话",
            Array.Empty<ChatTurn>(),
            null,
            now,
            now);

        _chatState = new ChatState(
            session.Id,
            [session, .. _chatState.Sessions]);
        _currentSessionId = session.Id;
        _attachments.Clear();
        ComposerBox.Text = string.Empty;
        RenderCurrentSession();
        RefreshAttachments();
        RefreshSessions();
        await SaveChatStateAsync();

        UpdateStatusBar("success", "已新建对话", "可以开始新的提问。");
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

    private async void OnAddImageClicked(object sender, RoutedEventArgs e)
    {
        try
        {
            var picker = new FileOpenPicker
            {
                SuggestedStartLocation = PickerLocationId.PicturesLibrary
            };
            picker.FileTypeFilter.Add(".png");
            picker.FileTypeFilter.Add(".jpg");
            picker.FileTypeFilter.Add(".jpeg");
            picker.FileTypeFilter.Add(".webp");
            picker.FileTypeFilter.Add(".gif");
            InitializeWithWindow.Initialize(picker, WindowNative.GetWindowHandle(this));

            var files = await picker.PickMultipleFilesAsync();
            if (files.Count == 0)
            {
                return;
            }

            foreach (var file in files)
            {
                if (_attachments.Any(item => item.Path == file.Path))
                {
                    continue;
                }

                _attachments.Add(new ChatAttachment(file.Path, file.Name));
            }

            RefreshAttachments();
            UpdateStatusBar("success", "图片已添加", $"当前已附加 {_attachments.Count} 张图片。");
        }
        catch (Exception error)
        {
            ShowError("添加图片失败", error);
        }
    }

    private async void OnComposerKeyDown(object sender, KeyRoutedEventArgs e)
    {
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

    private void OnComposerSendAcceleratorInvoked(
        KeyboardAccelerator sender,
        KeyboardAcceleratorInvokedEventArgs args)
    {
        var shiftState = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift);
        if (shiftState.HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down))
        {
            return;
        }

        args.Handled = true;
        _ = SendCurrentMessageAsync();
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

        ComposerBox.Text = string.Empty;
        _attachments.Clear();
        RefreshAttachments();

        try
        {
            SendButton.IsEnabled = false;
            AddImageButton.IsEnabled = false;
            UpdateStatusBar("info", "助手处理中", "正在准备请求...");

            await CompressCurrentSessionIfNeededAsync(prompt, pendingAttachments);
            var history = GetConversationHistory();
            AddTurn("user", userDisplay, attachments: pendingAttachments);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();

            var answer = await _backendClient.SendAsync<GroundedAnswer>(
                "askWithAi",
                new
                {
                    question = prompt,
                    history,
                    imagePaths = pendingAttachments.Select(item => item.Path).ToArray()
                });
            AddTurn("assistant", answer?.Answer ?? string.Empty, answer);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();

            if (answer?.SavedNote is not null)
            {
                AppendMessage("系统", $"已保存笔记：{answer.SavedNote.Title}");
                ScrollToLatest();
            }

            RestoreIdleStatus();
        }
        catch (Exception error)
        {
            var message = LocalizeError(error.Message);
            AddTurn("assistant", message);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();
            ShowError("请求失败", error, addMessage: false);
        }
        finally
        {
            SendButton.IsEnabled = true;
            AddImageButton.IsEnabled = true;
            RefreshSessions();
        }
    }

    private async Task RecordCurrentMessageAsync()
    {
        if (!SendButton.IsEnabled)
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

        ComposerBox.Text = string.Empty;
        _attachments.Clear();
        RefreshAttachments();

        try
        {
            SendButton.IsEnabled = false;
            RecordButton.IsEnabled = false;
            AddImageButton.IsEnabled = false;
            UpdateStatusBar("info", "正在记录知识", "正在整理并保存...");

            await CompressCurrentSessionIfNeededAsync(prompt, pendingAttachments);
            var history = GetConversationHistory();
            AddTurn("user", userDisplay, attachments: pendingAttachments);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();

            var answer = await _backendClient.SendAsync<GroundedAnswer>(
                "askWithAi",
                new
                {
                    question = prompt,
                    history,
                    imagePaths = pendingAttachments.Select(item => item.Path).ToArray()
                });
            if (answer?.SavedNote is null)
            {
                throw new InvalidOperationException("知识库写入未完成，模型未返回已保存笔记。");
            }
            var savedNote = answer.SavedNote;

            AddTurn("assistant", answer?.Answer ?? string.Empty, answer);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();

            AppendMessage("系统", $"已保存笔记：{savedNote.Title}");
            ScrollToLatest();
            var notes = await _backendClient.SendAsync<IReadOnlyList<NoteMeta>>("listNotes", new { });
            _noteCount = notes?.Count ?? 0;
            RefreshVaultSummary();

            RestoreIdleStatus("知识已记录", $"已保存为笔记：{savedNote.Title}");
        }
        catch (Exception error)
        {
            var message = LocalizeError(error.Message);
            AddTurn("assistant", message);
            RenderCurrentSession();
            ScrollToLatest();
            await SaveChatStateAsync();
            ShowError("记录失败", error, addMessage: false);
        }
        finally
        {
            SendButton.IsEnabled = true;
            RecordButton.IsEnabled = true;
            AddImageButton.IsEnabled = true;
            RefreshSessions();
        }
    }

    private async void OnClosed(object sender, WindowEventArgs args)
    {
        await _backendClient.DisposeAsync();
    }

    private void OnAgentStatusReceived(AgentStatusEvent status)
    {
        DispatcherQueue.TryEnqueue(() =>
        {
            UpdateStatusBar("info", LocalizeStage(status.Stage), LocalizeStatusDetail(status.Detail));
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
            Child = new TextBlock
            {
                Text = isUser || isAssistant ? text : $"{author}: {text}",
                TextWrapping = TextWrapping.Wrap,
                IsTextSelectionEnabled = !isUser,
                Foreground = isUser
                    ? (Brush)Application.Current.Resources["TextOnAccentFillColorPrimaryBrush"]
                    : (Brush)Application.Current.Resources["TextFillColorPrimaryBrush"]
            }
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

    private FrameworkElement CreateAttachmentChip(ChatAttachment attachment)
    {
        var preview = new Image
        {
            Width = 44,
            Height = 44,
            Stretch = Stretch.UniformToFill,
            Opacity = 0.2
        };
        _ = LoadImagePreviewAsync(preview, attachment.Path);

        var imageButton = new Button
        {
            Padding = new Thickness(0),
            MinWidth = 0,
            MinHeight = 0,
            HorizontalAlignment = HorizontalAlignment.Left,
            Content = preview
        };
        imageButton.Click += async (_, _) => await ShowImagePreviewDialogAsync(attachment);

        var label = new Button
        {
            Content = ShortenPath(attachment.Path),
            Padding = new Thickness(8, 4, 8, 4),
            HorizontalAlignment = HorizontalAlignment.Left
        };
        label.Click += async (_, _) => await ShowImagePreviewDialogAsync(attachment);

        var remove = new Button
        {
            Content = "移除",
            Padding = new Thickness(6, 4, 6, 4),
            HorizontalAlignment = HorizontalAlignment.Left
        };
        remove.Click += (_, _) =>
        {
            _attachments.RemoveAll(item => item.Path == attachment.Path);
            RefreshAttachments();
        };

        var stack = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 6,
            VerticalAlignment = VerticalAlignment.Center
        };
        stack.Children.Add(imageButton);
        stack.Children.Add(label);
        stack.Children.Add(remove);

        return new Border
        {
            CornerRadius = new CornerRadius(8),
            BorderThickness = new Thickness(1),
            BorderBrush = (Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"],
            Background = (Brush)Application.Current.Resources["CardBackgroundFillColorSecondaryBrush"],
            Padding = new Thickness(6),
            Child = stack
        };
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

    private async Task ShowImagePreviewDialogAsync(ChatAttachment attachment)
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
            CloseButtonText = "关闭"
        };
        await dialog.ShowAsync();
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
            "error" => new SolidColorBrush(Microsoft.UI.Colors.Red),
            "warning" => new SolidColorBrush(Microsoft.UI.Colors.Orange),
            "success" => new SolidColorBrush(Microsoft.UI.Colors.Green),
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
        }
        RefreshContextStatus();
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
        var sessions = _chatState.Sessions
            .Select(item => item.Id == updated.Id ? updated : item)
            .ToArray();
        _chatState = new ChatState(updated.Id, sessions);
        _currentSessionId = updated.Id;
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

        ContextRing.Stroke = remainingPercent switch
        {
            > 50 => new SolidColorBrush(Microsoft.UI.Colors.ForestGreen),
            > 20 => new SolidColorBrush(Microsoft.UI.Colors.Orange),
            _ => new SolidColorBrush(Microsoft.UI.Colors.Red)
        };

        ToolTipService.SetToolTip(ContextRing,
            $"上下文剩余：{remainingPercent:0.#}%（约 {FormatTokenCount(remainingTokens)} / {FormatTokenCount(contextWindow)}）");
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
        if (model.Contains("glm-5.1"))
        {
            return 200_000;
        }
        if (model.Contains("claude"))
        {
            return model.Contains("1m") ? 1_000_000UL : 200_000;
        }
        if (model.Contains("gpt-4.1") || model.Contains("gpt-5"))
        {
            return 1_047_576;
        }
        if (model.Contains("gpt-4o"))
        {
            return 128_000;
        }
        if (model.Contains("o1") || model.Contains("o3") || model.Contains("o4"))
        {
            return 200_000;
        }
        if (model.Contains("gemini"))
        {
            return 1_000_000;
        }

        return 128_000;
    }

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

        var turns = session.Turns.Concat(new[] { turn }).ToArray();
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

    private async Task SaveChatStateAsync()
    {
        try
        {
            _chatState = await _backendClient.SendAsync<ChatState>(
                "saveChatState",
                new { state = _chatState }) ?? _chatState;
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
            .Replace("Backend request failed.", "后端请求失败。", StringComparison.Ordinal);
    }

    private sealed record SessionListItem(string Id, string Title, string Detail);
}
