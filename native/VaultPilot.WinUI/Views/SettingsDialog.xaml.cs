using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Views;

/// <summary>
/// Settings dialog extracted from MainWindow. Displays provider configuration,
/// auto-wake options, and general preferences. Validates input before saving
/// and delegates the actual persistence to the caller via callbacks.
/// </summary>
public sealed partial class SettingsDialog : ContentDialog
{
    private readonly AppSettings _settings;
    private readonly Func<string, string[]> _getModelsForProvider;
    private readonly Func<string> _resolveDisplayVersion;
    private readonly Func<DateTime?> _getNextAutoWakeTime;
    private readonly Func<Task> _openVaultDirectoryAsync;
    private readonly Func<Task> _openProjectHomepageAsync;
    private readonly Action<string, Exception> _showError;
    private readonly Func<AppSettings, Task> _saveSettingsAsync;

    /// <summary>
    /// Creates a new settings dialog.
    /// </summary>
    /// <param name="xamlRoot">XamlRoot from the parent window, required for ContentDialog.ShowAsync.</param>
    /// <param name="settings">Current application settings to populate the dialog.</param>
    /// <param name="getModelsForProvider">Returns model names for the given base URL.</param>
    /// <param name="resolveDisplayVersion">Returns the display version string.</param>
    /// <param name="getNextAutoWakeTime">Returns the next scheduled auto-wake time, if any.</param>
    /// <param name="openVaultDirectoryAsync">Opens the vault directory picker.</param>
    /// <param name="openProjectHomepageAsync">Opens the project homepage URL.</param>
    /// <param name="showError">Shows an error dialog to the user.</param>
    /// <param name="saveSettingsAsync">Persists the updated settings. Throws on failure.</param>
    public SettingsDialog(
        XamlRoot xamlRoot,
        AppSettings settings,
        Func<string, string[]> getModelsForProvider,
        Func<string> resolveDisplayVersion,
        Func<DateTime?> getNextAutoWakeTime,
        Func<Task> openVaultDirectoryAsync,
        Func<Task> openProjectHomepageAsync,
        Action<string, Exception> showError,
        Func<AppSettings, Task> saveSettingsAsync)
    {
        _settings = settings;
        _getModelsForProvider = getModelsForProvider;
        _resolveDisplayVersion = resolveDisplayVersion;
        _getNextAutoWakeTime = getNextAutoWakeTime;
        _openVaultDirectoryAsync = openVaultDirectoryAsync;
        _openProjectHomepageAsync = openProjectHomepageAsync;
        _showError = showError;
        _saveSettingsAsync = saveSettingsAsync;

        InitializeComponent();
        XamlRoot = xamlRoot;

        LoadSettings();
        WireUpButtons();
    }

    // ──────────────────────────────────────────────
    //  Initialization
    // ──────────────────────────────────────────────

    private void LoadSettings()
    {
        // Provider section
        VaultBox.Text = _settings.VaultDir;
        ApiKeyBox.Password = _settings.Provider.ApiKey;
        BaseUrlBox.Text = _settings.Provider.BaseUrl;
        ModelBox.Text = _settings.Provider.Model;
        TimeoutBox.Text = _settings.Provider.RequestTimeoutMs.ToString();
        ContextWindowBox.Text = _settings.Provider.ContextWindowTokens?.ToString() ?? string.Empty;

        // General section
        AutoCheckUpdatesBox.IsChecked = _settings.AutoCheckUpdates;

        // Auto-wake section
        AutoWakeEnabledBox.IsChecked = _settings.AutoWakeEnabled;
        AutoWakeIntervalBox.Text = _settings.AutoWakeIntervalMinutes.ToString();

        // Populate wake model ComboBox
        AutoWakeModelBox.Items.Add(string.Empty);
        foreach (var model in _getModelsForProvider(_settings.Provider.BaseUrl))
        {
            AutoWakeModelBox.Items.Add(model);
        }
        if (string.IsNullOrEmpty(_settings.AutoWakeModel))
        {
            AutoWakeModelBox.SelectedIndex = 0;
        }
        else
        {
            AutoWakeModelBox.Text = _settings.AutoWakeModel;
        }

        AutoWakeStartTimeBox.Text = _settings.AutoWakeStartTime ?? string.Empty;
        AutoWakeEndTimeBox.Text = _settings.AutoWakeEndTime ?? string.Empty;

        // Next wake label
        UpdateNextWakeLabel();

        // Footer
        VersionLabel.Text = _resolveDisplayVersion();
    }

    private void WireUpButtons()
    {
        OpenVaultButton.Click += async (_, _) => await _openVaultDirectoryAsync();
        ProjectLinkButton.Click += async (_, _) => await _openProjectHomepageAsync();
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
        UpdateNextWakeLabel();
    }

    private void UpdateNextWakeLabel()
    {
        if (AutoWakeEnabledBox.IsChecked == true)
        {
            var next = _getNextAutoWakeTime();
            if (next.HasValue)
            {
                NextWakeLabel.Text = next.Value.Date == DateTime.Today
                    ? $"下次唤醒: {next.Value:HH:mm}"
                    : $"下次唤醒: {next.Value:MM/dd HH:mm}";
                return;
            }
        }
        NextWakeLabel.Text = string.Empty;
    }

    // ──────────────────────────────────────────────
    //  PrimaryButtonClick – validate then save
    //  (wired in XAML)
    // ──────────────────────────────────────────────

    private async void OnPrimaryButtonClick(ContentDialog sender, ContentDialogButtonClickEventArgs args)
    {
        // Validate and save BEFORE the dialog closes so the user never
        // loses input on a validation failure.  Setting args.Cancel = true
        // keeps the dialog open; only an error-free path lets it close.
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

            ulong? contextWindowTokens = null;
            if (!string.IsNullOrWhiteSpace(ContextWindowBox.Text))
            {
                if (!ulong.TryParse(ContextWindowBox.Text.Trim(), out var parsedContextWindow))
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
                ErrorInfoBar.Message = string.Join("\n", validationErrors);
                ErrorInfoBar.IsOpen = true;
                args.Cancel = true;
                return;
            }

            ErrorInfoBar.IsOpen = false;

            if (!ulong.TryParse(AutoWakeIntervalBox.Text?.Trim() ?? "30", out var autoWakeInterval) || autoWakeInterval == 0)
            {
                autoWakeInterval = 30;
            }

            var autoWakeModel = (AutoWakeModelBox.SelectedItem as string ?? AutoWakeModelBox.Text ?? string.Empty).Trim();

            var updated = new AppSettings(
                VaultBox.Text.Trim(),
                new ProviderConfig(
                    trimmedApiKey,
                    trimmedBaseUrl,
                    trimmedModel,
                    timeoutMs,
                    contextWindowTokens),
                AutoCheckUpdatesBox.IsChecked ?? true,
                AutoWakeEnabledBox.IsChecked ?? false,
                autoWakeInterval,
                autoWakeModel,
                trimmedWakeStart,
                trimmedWakeEnd);

            await _saveSettingsAsync(updated);
        }
        catch (Exception error)
        {
            _showError("保存设置失败", error);
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

    private static void SetFieldError(TextBox box, TextBlock errorBlock, string message)
    {
        box.BorderBrush = (Brush)Application.Current.Resources["StatusErrorBrush"];
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
