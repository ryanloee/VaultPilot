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
/// </summary>
public sealed partial class SettingsDialog : ContentDialog
{
    private static readonly SolidColorBrush _transparentBrush = new(Microsoft.UI.Colors.Transparent);
    private readonly Func<Task> _openVaultDirectoryAsync;
    private readonly Func<Task> _openProjectHomepageAsync;
    private readonly AppSettings _originalSettings;

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
    }

    // ──────────────────────────────────────────────
    //  Initialization
    // ──────────────────────────────────────────────

    private void LoadSettings(AppSettings settings, string[] models, string? nextWakeText, string versionText)
    {
        // Provider section
        VaultBox.Text = settings.VaultDir;
        ApiKeyBox.Password = settings.Provider.ApiKey;
        BaseUrlBox.Text = settings.Provider.BaseUrl;
        ModelBox.Text = settings.Provider.Model;
        TimeoutBox.Text = settings.Provider.RequestTimeoutMs.ToString();
        ContextWindowBox.Text = settings.Provider.ContextWindowTokens?.ToString() ?? string.Empty;

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
        if (string.IsNullOrEmpty(settings.AutoWakeModel))
        {
            AutoWakeModelBox.SelectedIndex = 0;
        }
        else
        {
            AutoWakeModelBox.Text = settings.AutoWakeModel;
        }

        AutoWakeStartTimeBox.Text = settings.AutoWakeStartTime ?? string.Empty;
        AutoWakeEndTimeBox.Text = settings.AutoWakeEndTime ?? string.Empty;

        // Next wake label
        NextWakeLabel.Text = nextWakeText ?? string.Empty;

        // Footer
        VersionLabel.Text = versionText;
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

    // ──────────────────────────────────────────────
    //  LostFocus validation (wired in XAML)
    // ──────────────────────────────────────────────

    private void OnTimeoutLostFocus(object sender, RoutedEventArgs e)
    {
        if (!ulong.TryParse(TimeoutBox.Text.Trim(), out var v) || v == 0)
            SetFieldError(TimeoutBox, TimeoutError, "超时必须是大于 0 的数字");
        else
            ClearFieldError(TimeoutBox, TimeoutError);
    }

    private void OnContextWindowLostFocus(object sender, RoutedEventArgs e)
    {
        var text = ContextWindowBox.Text.Trim();
        if (!string.IsNullOrEmpty(text) && !ulong.TryParse(text, out _))
            SetFieldError(ContextWindowBox, ContextWindowError, "Token 数必须是数字");
        else
            ClearFieldError(ContextWindowBox, ContextWindowError);
    }

    private void OnAutoWakeIntervalLostFocus(object sender, RoutedEventArgs e)
    {
        if (!ulong.TryParse(AutoWakeIntervalBox.Text.Trim(), out var v) || v == 0)
            SetFieldError(AutoWakeIntervalBox, AutoWakeIntervalError, "间隔必须是大于 0 的数字");
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
            var validationErrors = new List<string>();

            var trimmedApiKey = ApiKeyBox.Password.Trim();
            if (string.IsNullOrEmpty(trimmedApiKey))
            {
                validationErrors.Add("API Key 不能为空。");
            }

            var trimmedBaseUrl = BaseUrlBox.Text.Trim();
            if (string.IsNullOrEmpty(trimmedBaseUrl))
            {
                validationErrors.Add("接口地址不能为空。");
            }
            else if (!Uri.TryCreate(trimmedBaseUrl, UriKind.Absolute, out var parsedUri)
                     || (parsedUri.Scheme != "http" && parsedUri.Scheme != "https"))
            {
                validationErrors.Add("接口地址必须是有效的 http:// 或 https:// URL。");
            }

            var trimmedModel = ModelBox.Text.Trim();
            if (string.IsNullOrEmpty(trimmedModel))
            {
                validationErrors.Add("模型名称不能为空。");
            }

            var trimmedWakeStart = AutoWakeStartTimeBox.Text?.Trim() ?? string.Empty;
            if (!string.IsNullOrEmpty(trimmedWakeStart) && !TimeSpan.TryParse(trimmedWakeStart, out _))
            {
                validationErrors.Add("自动唤醒开始时间格式无效，请使用 HH:mm 格式。");
            }

            var trimmedWakeEnd = AutoWakeEndTimeBox.Text?.Trim() ?? string.Empty;
            if (!string.IsNullOrEmpty(trimmedWakeEnd) && !TimeSpan.TryParse(trimmedWakeEnd, out _))
            {
                validationErrors.Add("自动唤醒结束时间格式无效，请使用 HH:mm 格式。");
            }

            if (!ulong.TryParse(TimeoutBox.Text.Trim(), out var timeoutMs) || timeoutMs == 0)
            {
                validationErrors.Add("请求超时必须是大于 0 的数字。");
            }
            else if (timeoutMs > 300_000)
            {
                validationErrors.Add("请求超时不能超过 300,000 毫秒 (5 分钟)。");
            }

            ulong? contextWindowTokens = null;
            if (!string.IsNullOrWhiteSpace(ContextWindowBox.Text))
            {
                if (!ulong.TryParse(ContextWindowBox.Text.Trim(), out var parsedContextWindow))
                {
                    validationErrors.Add("上下文窗口 Token 数必须是数字。");
                }
                else if (parsedContextWindow > 2_000_000)
                {
                    validationErrors.Add("上下文窗口 Token 数不能超过 2,000,000。");
                }
                else
                {
                    contextWindowTokens = parsedContextWindow;
                }
            }

            ulong autoWakeInterval;
            if (!ulong.TryParse(AutoWakeIntervalBox.Text?.Trim() ?? "30", out autoWakeInterval) || autoWakeInterval == 0)
            {
                autoWakeInterval = 30;
            }
            else if (autoWakeInterval > 1440)
            {
                validationErrors.Add("自动唤醒间隔不能超过 1,440 分钟 (24 小时)。");
            }

            if (validationErrors.Count > 0)
            {
                ErrorInfoBar.Message = string.Join("\n", validationErrors);
                ErrorInfoBar.IsOpen = true;
                args.Cancel = true;
                return;
            }

            ErrorInfoBar.IsOpen = false;

            var autoWakeModel = (AutoWakeModelBox.SelectedItem as string ?? AutoWakeModelBox.Text ?? string.Empty).Trim();

            UpdatedSettings = new AppSettings(
                VaultBox.Text.Trim(),
                new ProviderConfig(
                    trimmedApiKey,
                    trimmedBaseUrl,
                    trimmedModel,
                    timeoutMs,
                    contextWindowTokens,
                    _originalSettings.Provider.MaxOutputTokens,
                    _originalSettings.Provider.ProviderType),
                AutoCheckUpdatesBox.IsChecked ?? true,
                AutoWakeEnabledBox.IsChecked ?? false,
                autoWakeInterval,
                autoWakeModel,
                trimmedWakeStart,
                trimmedWakeEnd);
        }
        catch (Exception error)
        {
            // Show error but let the dialog close — the caller checks UpdatedSettings == null.
            ErrorInfoBar.Message = $"保存设置失败：{error.Message}";
            ErrorInfoBar.IsOpen = true;
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
}
