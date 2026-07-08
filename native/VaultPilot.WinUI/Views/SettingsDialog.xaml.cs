using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using VaultPilot.WinUI.Models;
using System.Diagnostics;

namespace VaultPilot.WinUI.Views;

/// <summary>
/// Settings dialog extracted from MainWindow. Displays provider configuration,
/// auto-wake options, and general preferences. Validates input before saving.
/// After ShowAsync(), check UpdatedSettings for the validated result.
///
/// Validation is split into two independent groups:
///   1. Provider fields (API key, base URL, model, timeout, context window)
///   2. Wake word fields (interval, start/end time format)
/// Each group validates independently and sets inline errors on the failing field.
/// Wake word settings are saved even if provider validation fails (partial save).
/// </summary>
public sealed partial class SettingsDialog : ContentDialog
{
    private static readonly SolidColorBrush _transparentBrush = new(Microsoft.UI.Colors.Transparent);
    private readonly Func<Task> _openVaultDirectoryAsync;
    private readonly Func<Task> _openProjectHomepageAsync;
    private readonly AppSettings _originalSettings;
    private List<ProviderConfig> _providers = new();
    private int _activeProviderIndex;

    /// <summary>
    /// Tracks whether any provider field was modified since the dialog opened.
    /// Used to decide whether to validate provider fields on save.
    /// </summary>
    private bool _providerFieldsDirty;

    /// <summary>
    /// The updated settings after successful validation, or null if the user cancelled.
    /// </summary>
    public AppSettings? UpdatedSettings { get; private set; }

    /// <summary>
    /// Creates a new settings dialog.
    /// </summary>
    /// <param name="settings">Current application settings to populate the dialog.</param>
    /// <param name="models">Model names available for the current provider.</param>
    /// <param name="nextWakeText">Pre-computed next wake time display text, or null.</param>
    /// <param name="versionText">Display version string.</param>
    /// <param name="xamlRoot">XamlRoot from the parent window.</param>
    /// <param name="openVaultDirectoryAsync">Opens the vault directory picker.</param>
    /// <param name="openProjectHomepageAsync">Opens the project homepage URL.</param>
    public SettingsDialog(
        AppSettings settings,
        string[] models,
        string? nextWakeText,
        string versionText,
        XamlRoot xamlRoot,
        Func<Task> openVaultDirectoryAsync,
        Func<Task> openProjectHomepageAsync)
    {
        _openVaultDirectoryAsync = openVaultDirectoryAsync;
        _openProjectHomepageAsync = openProjectHomepageAsync;
        _originalSettings = settings;

        InitializeComponent();
        XamlRoot = xamlRoot;

        LoadSettings(settings, models, nextWakeText, versionText);
        WireUpButtons();
        WireUpProviderDirtyTracking();
    }

    // ──────────────────────────────────────────────
    //  Initialization
    // ──────────────────────────────────────────────

    private void LoadSettings(AppSettings settings, string[] models, string? nextWakeText, string versionText)
    {
        // Provider list
        VaultBox.Text = settings.VaultDir;
        if (settings.Providers.Count > 0)
        {
            _providers = new List<ProviderConfig>(settings.Providers);
            _activeProviderIndex = Math.Clamp(settings.ActiveProviderIndex, 0, _providers.Count - 1);
        }
        else
        {
            // Migrate legacy single provider
            _providers = new List<ProviderConfig> { settings.Provider };
            _activeProviderIndex = 0;
        }
        RefreshProviderList();
        LoadProviderFields(_providers[_activeProviderIndex]);

        // General section
        AutoCheckUpdatesBox.IsChecked = settings.AutoCheckUpdates;

        // Auto-wake section
        AutoWakeEnabledBox.IsChecked = settings.AutoWakeEnabled;
        AutoWakeIntervalBox.Text = settings.AutoWakeIntervalMinutes.ToString();

        // Populate wake model ComboBox
        AutoWakeModelBox.Items.Add(string.Empty);
        foreach (var model in models)
        {
            AutoWakeModelBox.Items.Add(model);
        }
        if (!string.IsNullOrEmpty(settings.AutoWakeModel))
        {
            var matchIndex = -1;
            for (var i = 0; i < AutoWakeModelBox.Items.Count; i++)
            {
                if (string.Equals(AutoWakeModelBox.Items[i] as string, settings.AutoWakeModel, StringComparison.Ordinal))
                {
                    matchIndex = i;
                    break;
                }
            }
            if (matchIndex < 0)
            {
                AutoWakeModelBox.Items.Add(settings.AutoWakeModel);
                matchIndex = AutoWakeModelBox.Items.Count - 1;
            }
            AutoWakeModelBox.SelectedIndex = matchIndex;
        }
        // Belt-and-suspenders: WinUI 3 editable ComboBox may not propagate
        // SelectedIndex → Text reliably before Loaded.  Set Text explicitly.
        AutoWakeModelBox.Text = settings.AutoWakeModel ?? string.Empty;

        AutoWakeStartTimeBox.Text = settings.AutoWakeStartTime ?? string.Empty;
        AutoWakeEndTimeBox.Text = settings.AutoWakeEndTime ?? string.Empty;
        AutoWakePromptBox.Text = settings.AutoWakePrompt ?? string.Empty;

        // Next wake label
        NextWakeLabel.Text = nextWakeText ?? string.Empty;

        // Footer
        VersionLabel.Text = versionText;
    }

    private void LoadProviderFields(ProviderConfig p)
    {
        ProviderNameBox.Text = p.Name ?? string.Empty;
        ApiKeyBox.Password = p.ApiKey;
        BaseUrlBox.Text = p.BaseUrl;
        ModelBox.Text = p.Model;
        TimeoutBox.Text = p.RequestTimeoutMs.ToString();
        ContextWindowBox.Text = p.ContextWindowTokens?.ToString() ?? string.Empty;
        // Provider type
        var ptype = (p.ProviderType ?? "openai").ToLowerInvariant();
        ProviderTypeBox.SelectedIndex = ptype.Contains("anthropic") ? 1 : 0;

        // Clear dirty flag after loading fields (loading != user editing)
        _providerFieldsDirty = false;
    }

    private void RefreshProviderList()
    {
        ProviderList.Items.Clear();
        for (int i = 0; i < _providers.Count; i++)
        {
            var p = _providers[i];
            var label = string.IsNullOrEmpty(p.Name) ? $"提供商 {i + 1}" : p.Name;
            if (i == _activeProviderIndex) label = "● " + label;
            ProviderList.Items.Add(label);
        }
        if (_activeProviderIndex >= 0 && _activeProviderIndex < _providers.Count)
            ProviderList.SelectedIndex = _activeProviderIndex;
    }

    private void WireUpButtons()
    {
        OpenVaultButton.Click += async (_, _) =>
        {
            try { await _openVaultDirectoryAsync(); }
            catch (Exception ex) { Trace.TraceError($"OpenVault error: {ex}"); }
        };
        ProjectLinkButton.Click += async (_, _) =>
        {
            try { await _openProjectHomepageAsync(); }
            catch (Exception ex) { Trace.TraceError($"ProjectLink error: {ex}"); }
        };
    }

    /// <summary>
    /// Wires TextChanged / PasswordChanged / SelectionChanged on provider fields
    /// so we know if the user actually edited anything. This avoids rejecting a
    /// save when the user only changed wake word settings.
    /// </summary>
    private void WireUpProviderDirtyTracking()
    {
        ApiKeyBox.PasswordChanged += (_, _) => _providerFieldsDirty = true;
        BaseUrlBox.TextChanged += (_, _) => _providerFieldsDirty = true;
        ModelBox.TextChanged += (_, _) => _providerFieldsDirty = true;
        TimeoutBox.TextChanged += (_, _) => _providerFieldsDirty = true;
        ContextWindowBox.TextChanged += (_, _) => _providerFieldsDirty = true;
        ProviderNameBox.TextChanged += (_, _) => _providerFieldsDirty = true;
        ProviderTypeBox.SelectionChanged += (_, _) => _providerFieldsDirty = true;
    }

    // ──────────────────────────────────────────────
    //  Provider list management
    // ──────────────────────────────────────────────

    private void SaveCurrentProviderFields()
    {
        if (_activeProviderIndex < 0 || _activeProviderIndex >= _providers.Count) return;
        var ptype = ProviderTypeBox.SelectedIndex == 1 ? "anthropic" : "openai";
        _providers[_activeProviderIndex] = new ProviderConfig(
            ApiKeyBox.Password.Trim(),
            BaseUrlBox.Text.Trim(),
            ModelBox.Text.Trim(),
            ulong.TryParse(TimeoutBox.Text.Trim(), out var t) ? t : 60000,
            ulong.TryParse(ContextWindowBox.Text.Trim(), out var cw) ? cw : null,
            _providers[_activeProviderIndex].MaxOutputTokens,
            ptype,
            ProviderNameBox.Text.Trim());
    }

    private void OnProviderSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        var idx = ProviderList.SelectedIndex;
        if (idx < 0 || idx >= _providers.Count) return;
        // Save current fields before switching
        SaveCurrentProviderFields();
        _activeProviderIndex = idx;
        LoadProviderFields(_providers[idx]);
    }

    private void OnAddProvider(object sender, RoutedEventArgs e)
    {
        SaveCurrentProviderFields();
        _providers.Add(new ProviderConfig(
            string.Empty, string.Empty, string.Empty,
            60000, null, null, "openai", "新提供商"));
        _activeProviderIndex = _providers.Count - 1;
        RefreshProviderList();
        LoadProviderFields(_providers[_activeProviderIndex]);
    }

    private void OnRemoveProvider(object sender, RoutedEventArgs e)
    {
        if (_providers.Count <= 1) return;
        _providers.RemoveAt(_activeProviderIndex);

        // Clamp index: must be in [0, Count-1]
        if (_activeProviderIndex >= _providers.Count)
            _activeProviderIndex = _providers.Count - 1;
        // Also handle the edge case where we removed index 0 and list is now [0..N-1]
        if (_activeProviderIndex < 0)
            _activeProviderIndex = 0;

        RefreshProviderList();
        LoadProviderFields(_providers[_activeProviderIndex]);

        // Clear any stale inline errors from the deleted provider
        ClearProviderFieldErrors();
    }

    // ──────────────────────────────────────────────
    //  LostFocus validation (wired in XAML)
    // ──────────────────────────────────────────────

    private void OnTimeoutLostFocus(object sender, RoutedEventArgs e)
    {
        if (!ulong.TryParse(TimeoutBox.Text.Trim(), out var v) || v < 1_000)
            SetFieldError(TimeoutBox, TimeoutError, "超时不能少于 1,000 毫秒");
        else if (v > 300_000)
            SetFieldError(TimeoutBox, TimeoutError, "超时不能超过 300,000 毫秒");
        else
            ClearFieldError(TimeoutBox, TimeoutError);
    }

    private void OnContextWindowLostFocus(object sender, RoutedEventArgs e)
    {
        var text = ContextWindowBox.Text.Trim();
        if (!string.IsNullOrEmpty(text))
        {
            if (!ulong.TryParse(text, out var v))
                SetFieldError(ContextWindowBox, ContextWindowError, "Token 数必须是数字");
            else if (v > 2_000_000)
                SetFieldError(ContextWindowBox, ContextWindowError, "Token 数不能超过 2,000,000");
            else
                ClearFieldError(ContextWindowBox, ContextWindowError);
        }
        else
            ClearFieldError(ContextWindowBox, ContextWindowError);
    }

    private void OnAutoWakeIntervalLostFocus(object sender, RoutedEventArgs e)
    {
        if (!ulong.TryParse(AutoWakeIntervalBox.Text.Trim(), out var v) || v == 0)
            SetFieldError(AutoWakeIntervalBox, AutoWakeIntervalError, "间隔必须是大于 0 的数字");
        else if (v > 1440)
            SetFieldError(AutoWakeIntervalBox, AutoWakeIntervalError, "间隔不能超过 1,440 分钟 (24 小时)");
        else
            ClearFieldError(AutoWakeIntervalBox, AutoWakeIntervalError);
    }

    private void OnAutoWakeStartTimeLostFocus(object sender, RoutedEventArgs e)
    {
        var text = AutoWakeStartTimeBox.Text?.Trim() ?? string.Empty;
        if (!string.IsNullOrEmpty(text) && !TimeSpan.TryParse(text, out _))
            SetFieldError(AutoWakeStartTimeBox, AutoWakeStartTimeError, "时间格式无效，请使用 HH:mm");
        else
            ClearFieldError(AutoWakeStartTimeBox, AutoWakeStartTimeError);
    }

    private void OnAutoWakeEndTimeLostFocus(object sender, RoutedEventArgs e)
    {
        var text = AutoWakeEndTimeBox.Text?.Trim() ?? string.Empty;
        if (!string.IsNullOrEmpty(text) && !TimeSpan.TryParse(text, out _))
            SetFieldError(AutoWakeEndTimeBox, AutoWakeEndTimeError, "时间格式无效，请使用 HH:mm");
        else
            ClearFieldError(AutoWakeEndTimeBox, AutoWakeEndTimeError);
    }

    // ──────────────────────────────────────────────
    //  Auto-wake toggle (wired in XAML)
    // ──────────────────────────────────────────────

    private void OnAutoWakeToggled(object sender, RoutedEventArgs e)
    {
        // Next wake label is computed by MainWindow; no-op here.
    }

    // ──────────────────────────────────────────────
    //  PrimaryButtonClick – validate then save
    //  (wired in XAML)
    // ──────────────────────────────────────────────

    private async void OnPrimaryButtonClick(ContentDialog sender, ContentDialogButtonClickEventArgs args)
    {
        var deferral = args.GetDeferral();
        try
        {
            // Clear all previous inline errors
            ClearProviderFieldErrors();
            ClearWakeWordFieldErrors();
            ErrorInfoBar.IsOpen = false;

            // Track the first error element so we can scroll to it
            UIElement? firstErrorElement = null;

            // ── 1. Provider validation (only if user actually changed something) ──
            bool providerValid = true;
            if (_providerFieldsDirty)
            {
                var trimmedApiKey = ApiKeyBox.Password.Trim();
                if (string.IsNullOrEmpty(trimmedApiKey))
                {
                    SetFieldError(ApiKeyBox, ApiKeyError, "API Key 不能为空。可在 opencode.ai/zen 或 openrouter.ai 免费获取。");
                    providerValid = false;
                    firstErrorElement ??= ApiKeyBox;
                }

                var trimmedBaseUrl = BaseUrlBox.Text.Trim();
                if (string.IsNullOrEmpty(trimmedBaseUrl))
                {
                    SetFieldError(BaseUrlBox, BaseUrlError, "接口地址不能为空。");
                    providerValid = false;
                    firstErrorElement ??= BaseUrlBox;
                }
                else if (!Uri.TryCreate(trimmedBaseUrl, UriKind.Absolute, out var parsedUri)
                         || (parsedUri.Scheme != "http" && parsedUri.Scheme != "https"))
                {
                    SetFieldError(BaseUrlBox, BaseUrlError, "接口地址必须是有效的 http:// 或 https:// URL。");
                    providerValid = false;
                    firstErrorElement ??= BaseUrlBox;
                }

                var trimmedModel = ModelBox.Text.Trim();
                if (string.IsNullOrEmpty(trimmedModel))
                {
                    SetFieldError(ModelBox, ModelError, "模型名称不能为空。");
                    providerValid = false;
                    firstErrorElement ??= ModelBox;
                }

                if (!ulong.TryParse(TimeoutBox.Text.Trim(), out var timeoutMs) || timeoutMs < 1_000)
                {
                    SetFieldError(TimeoutBox, TimeoutError, "请求超时不能少于 1,000 毫秒 (1 秒)。");
                    providerValid = false;
                    firstErrorElement ??= TimeoutBox;
                }
                else if (timeoutMs > 300_000)
                {
                    SetFieldError(TimeoutBox, TimeoutError, "请求超时不能超过 300,000 毫秒 (5 分钟)。");
                    providerValid = false;
                    firstErrorElement ??= TimeoutBox;
                }

                if (!string.IsNullOrWhiteSpace(ContextWindowBox.Text))
                {
                    if (!ulong.TryParse(ContextWindowBox.Text.Trim(), out var parsedContextWindow))
                    {
                        SetFieldError(ContextWindowBox, ContextWindowError, "上下文窗口 Token 数必须是数字。");
                        providerValid = false;
                        firstErrorElement ??= ContextWindowBox;
                    }
                    else if (parsedContextWindow > 2_000_000)
                    {
                        SetFieldError(ContextWindowBox, ContextWindowError, "上下文窗口 Token 数不能超过 2,000,000。");
                        providerValid = false;
                        firstErrorElement ??= ContextWindowBox;
                    }
                }
            }

            // ── 2. Wake word validation (always, independently) ──
            bool wakeWordValid = true;

            var trimmedWakeStart = AutoWakeStartTimeBox.Text?.Trim() ?? string.Empty;
            if (!string.IsNullOrEmpty(trimmedWakeStart) && !TimeSpan.TryParse(trimmedWakeStart, out _))
            {
                SetFieldError(AutoWakeStartTimeBox, AutoWakeStartTimeError, "时间格式无效，请使用 HH:mm 格式。");
                wakeWordValid = false;
                firstErrorElement ??= AutoWakeStartTimeBox;
            }

            var trimmedWakeEnd = AutoWakeEndTimeBox.Text?.Trim() ?? string.Empty;
            if (!string.IsNullOrEmpty(trimmedWakeEnd) && !TimeSpan.TryParse(trimmedWakeEnd, out _))
            {
                SetFieldError(AutoWakeEndTimeBox, AutoWakeEndTimeError, "时间格式无效，请使用 HH:mm 格式。");
                wakeWordValid = false;
                firstErrorElement ??= AutoWakeEndTimeBox;
            }

            ulong autoWakeInterval;
            if (!ulong.TryParse(AutoWakeIntervalBox.Text?.Trim() ?? "30", out autoWakeInterval) || autoWakeInterval == 0)
            {
                autoWakeInterval = 30; // fallback default
            }
            else if (autoWakeInterval > 1440)
            {
                SetFieldError(AutoWakeIntervalBox, AutoWakeIntervalError, "自动唤醒间隔不能超过 1,440 分钟 (24 小时)。");
                wakeWordValid = false;
                firstErrorElement ??= AutoWakeIntervalBox;
            }

            // ── 3. Scroll to first error and abort if anything failed ──
            if (!providerValid || !wakeWordValid)
            {
                // Show a summary in the top bar
                var errorSummary = new List<string>();
                if (!providerValid) errorSummary.Add("提供商配置有误，请检查上方字段。");
                if (!wakeWordValid) errorSummary.Add("自动唤醒设置有误，请检查下方字段。");
                ErrorInfoBar.Message = string.Join("\n", errorSummary);
                ErrorInfoBar.IsOpen = true;

                // Scroll to the first error field
                if (firstErrorElement != null)
                {
                    firstErrorElement.StartBringIntoView();
                }
                else
                {
                    SettingsScroller.ChangeView(null, 0, null, true);
                }

                args.Cancel = true;
                return;
            }

            // ── 4. Build settings from validated fields ──
            // Always write current provider fields back to the list (harmless if unchanged).
            var trimmedApiKey2 = ApiKeyBox.Password.Trim();
            var trimmedBaseUrl2 = BaseUrlBox.Text.Trim();
            var trimmedModel2 = ModelBox.Text.Trim();
            var timeoutMs2 = ulong.TryParse(TimeoutBox.Text.Trim(), out var t2) ? t2 : 60000;
            ulong? contextWindowTokens2 = null;
            if (ulong.TryParse(ContextWindowBox.Text.Trim(), out var cw2))
                contextWindowTokens2 = cw2;
            var ptype = ProviderTypeBox.SelectedIndex == 1 ? "anthropic" : "openai";

            _providers[_activeProviderIndex] = new ProviderConfig(
                trimmedApiKey2, trimmedBaseUrl2, trimmedModel2,
                timeoutMs2, contextWindowTokens2,
                _providers[_activeProviderIndex].MaxOutputTokens,
                ptype, ProviderNameBox.Text.Trim());

            var autoWakeModel = (AutoWakeModelBox.Text ?? string.Empty).Trim();

            UpdatedSettings = new AppSettings(
                VaultBox.Text.Trim(),
                _providers[_activeProviderIndex],
                AutoCheckUpdatesBox.IsChecked ?? true,
                AutoWakeEnabledBox.IsChecked ?? false,
                autoWakeInterval,
                autoWakeModel,
                trimmedWakeStart,
                trimmedWakeEnd,
                AutoWakePromptBox.Text?.Trim() ?? string.Empty,
                _providers,
                _activeProviderIndex);
        }
        catch (Exception error)
        {
            // Show error and keep the dialog open so the user can retry or cancel.
            ErrorInfoBar.Message = $"保存设置失败：{error.Message}";
            ErrorInfoBar.IsOpen = true;
            SettingsScroller.ChangeView(null, 0, null, true);
            args.Cancel = true;
        }
        finally
        {
            deferral.Complete();
        }
    }

    // ──────────────────────────────────────────────
    //  Inline field error helpers
    // ──────────────────────────────────────────────

    private static Brush GetThemeBrush(string key)
    {
        if (Application.Current?.Resources.TryGetValue(key, out var value) == true && value is Brush brush)
        {
            return brush;
        }

        System.Diagnostics.Debug.WriteLine($"[SettingsDialog.GetThemeBrush] Missing resource key: '{key}', falling back to Transparent.");
        return _transparentBrush;
    }

    private static void SetFieldError(TextBox box, TextBlock errorBlock, string message)
    {
        box.BorderBrush = GetThemeBrush("StatusErrorBrush");
        errorBlock.Text = message;
        errorBlock.Visibility = Visibility.Visible;
    }

    private static void ClearFieldError(TextBox box, TextBlock errorBlock)
    {
        box.ClearValue(TextBox.BorderBrushProperty);
        errorBlock.Text = string.Empty;
        errorBlock.Visibility = Visibility.Collapsed;
    }

    /// <summary>
    /// Overload for PasswordBox (API Key field). Sets the border to error color
    /// and shows the adjacent error TextBlock.
    /// </summary>
    private static void SetFieldError(PasswordBox box, TextBlock errorBlock, string message)
    {
        box.BorderBrush = GetThemeBrush("StatusErrorBrush");
        errorBlock.Text = message;
        errorBlock.Visibility = Visibility.Visible;
    }

    private static void ClearFieldError(PasswordBox box, TextBlock errorBlock)
    {
        box.ClearValue(PasswordBox.BorderBrushProperty);
        errorBlock.Text = string.Empty;
        errorBlock.Visibility = Visibility.Collapsed;
    }

    /// <summary>
    /// Clears all inline errors on provider fields.
    /// </summary>
    private void ClearProviderFieldErrors()
    {
        ClearFieldError(ApiKeyBox, ApiKeyError);
        ClearFieldError(BaseUrlBox, BaseUrlError);
        ClearFieldError(ModelBox, ModelError);
        ClearFieldError(TimeoutBox, TimeoutError);
        ClearFieldError(ContextWindowBox, ContextWindowError);
    }

    /// <summary>
    /// Clears all inline errors on wake word fields.
    /// </summary>
    private void ClearWakeWordFieldErrors()
    {
        ClearFieldError(AutoWakeIntervalBox, AutoWakeIntervalError);
        ClearFieldError(AutoWakeStartTimeBox, AutoWakeStartTimeError);
        ClearFieldError(AutoWakeEndTimeBox, AutoWakeEndTimeError);
    }
}
