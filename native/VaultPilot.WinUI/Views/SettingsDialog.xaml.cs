using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using VaultPilot.WinUI.Models;
using System.Collections.Generic;
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
    /// The updated settings after successful validation, or null if the user cancelled.
    /// </summary>
    public AppSettings? UpdatedSettings { get; private set; }

    /// <summary>
    /// The theme selected in the dialog. Set after a successful save; defaults
    /// to <see cref="ElementTheme.Default"/> (follow system) before that.
    /// MainWindow reads this to apply the theme immediately.
    /// </summary>
    public ElementTheme ThemeMode { get; private set; } = ElementTheme.Default;

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

        // Theme selector — persisted client-side (ThemePreferences), independent
        // of the backend AppSettings so it can ship without a schema change.
        var theme = ThemePreferences.Load();
        switch (theme)
        {
            case ElementTheme.Light:
                ThemeLight.IsChecked = true;
                break;
            case ElementTheme.Dark:
                ThemeDark.IsChecked = true;
                break;
            default:
                ThemeSystem.IsChecked = true;
                break;
        }
    }

    private void LoadProviderFields(ProviderConfig p)
    {
        ProviderNameBox.Text = p.Name ?? string.Empty;
        ApiKeyBox.Password = p.ApiKey;
        BaseUrlBox.Text = p.BaseUrl;
        ModelBox.Text = p.Model;
        TimeoutBox.Text = p.RequestTimeoutMs.ToString();
        ContextWindowBox.Text = p.ContextWindowTokens?.ToString() ?? string.Empty;
        // Provider type — round-trip the actual saved value so a non-OpenAI /
        // non-Anthropic provider (e.g. ollama) is preserved on save (#3131).
        var ptype = (p.ProviderType ?? "openai").ToLowerInvariant();
        ProviderTypeBox.SelectedIndex = ptype.Contains("anthropic") ? 1
            : ptype.Contains("ollama") ? 2
            : 0;
    }

    // Maps the ProviderTypeBox selection back to the canonical provider type
    // string so it round-trips through save without being rewritten (#3131).
    private static string SelectedProviderType(int selectedIndex) =>
        selectedIndex == 1 ? "anthropic"
        : selectedIndex == 2 ? "ollama"
        : "openai";

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

        // #3120 — keyboard navigation: Ctrl+F to re-focus search, ArrowUp/Down
        // to move between visible settings cards, Escape to clear search or
        // close the dialog. PreviewKeyDown fires before child controls handle
        // the key, so Ctrl+F still works even when focus is inside a TextBox.
        PreviewKeyDown += OnDialogPreviewKeyDown;
    }

    // ──────────────────────────────────────────────
    //  Keyboard navigation (#3120)
    // ──────────────────────────────────────────────

    /// <summary>
    /// Index of the currently keyboard-focused settings card (-1 = none).
    /// Refers to a position in the list returned by <see cref="GetVisibleSettingsCards"/>.
    /// </summary>
    private int _keyboardFocusCardIndex = -1;

    /// <summary>
    /// Top-level dialog PreviewKeyDown handler (#3120). Implements the keyboard
    /// contract promised in the issue:
    /// <list type="bullet">
    ///   <item><b>Ctrl+F</b> / <b>Cmd+F</b> — re-focus the settings search box
    ///     (even when focus is inside another input field).</item>
    ///   <item><b>Escape</b> — if the search box has text, clear it; otherwise
    ///     close the dialog.</item>
    ///   <item><b>ArrowUp</b> / <b>ArrowDown</b> — move keyboard focus between
    ///     currently-visible settings cards. No-op if no cards are visible
    ///     (e.g. search returned zero matches).</item>
    ///   <item><b>j</b> / <b>k</b> — Vim equivalents of ArrowDown / ArrowUp.
    ///     Only triggered when the focused element is NOT a text input so we
    ///     don't swallow legitimate typing.</item>
    /// </list>
    /// </summary>
    private void OnDialogPreviewKeyDown(object sender, KeyRoutedEventArgs e)
    {
        // NOTE: InputKeyboardSource.GetKeyStateForCurrentThread returns
        // Windows.UI.Core.CoreVirtualKeyStates in this Windows App SDK version
        // (see MainWindow.ChatInputHandler.cs for the established pattern).
        var ctrl = Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Control).HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down);
        var menu = Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Menu).HasFlag(Windows.UI.Core.CoreVirtualKeyStates.Down);
        var modifierPressed = ctrl || menu;

        switch (e.Key)
        {
            // Ctrl+F / Cmd+F: jump back to search box from anywhere in the dialog
            case Windows.System.VirtualKey.F when modifierPressed:
                e.Handled = true;
                FocusSearchBox();
                break;

            case Windows.System.VirtualKey.Escape:
                e.Handled = true;
                ClearSearchOrClose();
                break;

            case Windows.System.VirtualKey.Down:
                // Don't hijack arrow keys when the user is editing text — they
                // need them for caret movement. Enter / ArrowDown on the search
                // box is handled separately in OnSearchBoxKeyDown.
                if (!IsFocusInsideTextInput())
                {
                    e.Handled = true;
                    MoveCardFocus(delta: 1);
                }
                break;

            case Windows.System.VirtualKey.Up:
                if (!IsFocusInsideTextInput())
                {
                    e.Handled = true;
                    MoveCardFocus(delta: -1);
                }
                break;
        }

        // Optional Vim-style bindings (j/k = ↓/↑). Only active when focus is
        // not in a text input — otherwise typing 'j' into the API key field
        // would jump cards. This mirrors Obsidian 1.13.0's Vim-mode behaviour.
        if (!e.Handled && !IsFocusInsideTextInput())
        {
            if (e.Key == Windows.System.VirtualKey.J)
            {
                e.Handled = true;
                MoveCardFocus(delta: 1);
            }
            else if (e.Key == Windows.System.VirtualKey.K)
            {
                e.Handled = true;
                MoveCardFocus(delta: -1);
            }
        }
    }

    /// <summary>
    /// Keydown handler wired directly on SettingsSearchBox (#3120).
    /// Handles keys meaningful only while the search box itself has focus:
    /// <list type="bullet">
    ///   <item><b>Escape</b> — clear search text but keep focus on the box,
    ///     so the user can immediately type a new query. If the search is
    ///     already empty, fall through to the dialog-level handler which
    ///     will close the dialog.</item>
    ///   <item><b>Enter</b> / <b>ArrowDown</b> — drop into the first visible
    ///     settings card so the user can begin editing without reaching for
    ///     the mouse.</item>
    /// </list>
    /// </summary>
    private void OnSearchBoxKeyDown(object sender, KeyRoutedEventArgs e)
    {
        switch (e.Key)
        {
            case Windows.System.VirtualKey.Escape:
                if (!string.IsNullOrEmpty(SettingsSearchBox.Text))
                {
                    e.Handled = true;
                    SettingsSearchBox.Text = string.Empty;
                    // TextChanged will restore all cards; keep focus on search.
                }
                // else: let PreviewKeyDown close the dialog.
                break;

            case Windows.System.VirtualKey.Enter:
            case Windows.System.VirtualKey.Down:
                e.Handled = true;
                // Drop into the first visible card.
                _keyboardFocusCardIndex = -1;
                MoveCardFocus(delta: 1);
                break;
        }
    }

    /// <summary>
    /// Returns whether the currently-focused element is a text-input control.
    /// Used to detect whether the user is editing text (we shouldn't hijack
    /// typing keys for navigation in that case) — #3120.
    /// </summary>
    private static bool IsFocusInsideTextInput()
    {
        var focused = FocusManager.GetFocusedElement();
        return focused is TextBox or PasswordBox or ComboBox;
    }

    /// <summary>
    /// Sets keyboard focus to the settings search box and selects all text
    /// so a new query can be typed immediately (#3120).
    /// </summary>
    private void FocusSearchBox()
    {
        SettingsSearchBox.Focus(FocusState.Programmatic);
        SettingsSearchBox.SelectAll();
    }

    /// <summary>
    /// If the search box contains text, clear it (which restores all cards
    /// via OnSettingsSearchTextChanged). Otherwise, close the dialog (#3120).
    /// </summary>
    private void ClearSearchOrClose()
    {
        if (!string.IsNullOrEmpty(SettingsSearchBox.Text))
        {
            SettingsSearchBox.Text = string.Empty;
            SettingsSearchBox.Focus(FocusState.Programmatic);
        }
        else
        {
            Hide();
        }
    }

    /// <summary>
    /// Collects the settings cards (Border elements directly inside Panel)
    /// that are currently visible after search filtering. Cards are returned
    /// in document order so index-based navigation stays stable (#3120).
    /// </summary>
    private List<Border> GetVisibleSettingsCards()
    {
        var cards = new List<Border>();
        foreach (var child in Panel.Children)
        {
            if (child is Border card && card.Visibility == Visibility.Visible)
            {
                cards.Add(card);
            }
        }
        return cards;
    }

    /// <summary>
    /// Moves keyboard focus between visible settings cards by <paramref name="delta"/>
    /// (negative = up, positive = down). Wraps around at the top/bottom. Sets
    /// focus on the first focusable control inside the target card and brings
    /// it into view via StartBringIntoView (#3120).
    /// </summary>
    private void MoveCardFocus(int delta)
    {
        var cards = GetVisibleSettingsCards();
        if (cards.Count == 0) return;

        // Compute wrapped index. If no card is currently focused (index -1),
        // delta=+1 selects the first card, delta=-1 selects the last.
        int newIndex;
        if (_keyboardFocusCardIndex < 0 || _keyboardFocusCardIndex >= cards.Count)
        {
            newIndex = delta > 0 ? 0 : cards.Count - 1;
        }
        else
        {
            newIndex = (_keyboardFocusCardIndex + delta + cards.Count) % cards.Count;
        }
        _keyboardFocusCardIndex = newIndex;

        var target = cards[newIndex];
        // Find the first focusable control inside the card so arrow navigation
        // immediately enables editing (Obsidian 1.13.0 Enter-to-open semantics).
        var firstFocusable = FindFirstFocusableControl(target);
        if (firstFocusable is Control focusable)
        {
            focusable.Focus(FocusState.Programmatic);
        }
        else
        {
            // Card itself isn't focusable by default — at minimum bring it into
            // view so the user can see which card is logically "current".
            target.StartBringIntoView();
        }
    }

    /// <summary>
    /// Depth-first search for the first focusable Control inside a subtree.
    /// Used by <see cref="MoveCardFocus"/> to drop the user directly into the
    /// first editable field of a card (#3120).
    /// </summary>
    private static Control? FindFirstFocusableControl(DependencyObject root)
    {
        if (root is Control c && c.IsEnabled && c.Visibility == Visibility.Visible)
        {
            return c;
        }
        var count = VisualTreeHelper.GetChildrenCount(root);
        for (var i = 0; i < count; i++)
        {
            var child = VisualTreeHelper.GetChild(root, i);
            if (FindFirstFocusableControl(child) is { } found)
            {
                return found;
            }
        }
        return null;
    }

    // ──────────────────────────────────────────────
    //  Provider list management
    // ──────────────────────────────────────────────

    /// <summary>
    /// Pure, stateless validation + construction of a ProviderConfig from raw
    /// field strings. Shared by OnPrimaryButtonClick (save validation) and
    /// SaveCurrentProviderFields (per-switch persistence) so the rules never
    /// drift. Returns false (config null) when any field is invalid.
    /// Fixes #2781: provider validation must not be bypassed by a dirty flag,
    /// and illegal values typed before switching providers must not be written
    /// into _providers (where they survived into saved settings).
    /// </summary>
    public static bool TryBuildProviderConfig(
        string? apiKey,
        string? baseUrl,
        string? model,
        string? timeoutText,
        string? contextWindowText,
        uint? existingMaxOutputTokens,
        string? providerType,
        string? name,
        out ProviderConfig? config)
    {
        config = null;

        var trimmedApiKey = (apiKey ?? string.Empty).Trim();
        if (string.IsNullOrEmpty(trimmedApiKey)) return false;

        var trimmedBaseUrl = (baseUrl ?? string.Empty).Trim();
        if (string.IsNullOrEmpty(trimmedBaseUrl)) return false;
        if (!Uri.TryCreate(trimmedBaseUrl, UriKind.Absolute, out var parsedUri)
            || (parsedUri.Scheme != "http" && parsedUri.Scheme != "https")) return false;

        var trimmedModel = (model ?? string.Empty).Trim();
        if (string.IsNullOrEmpty(trimmedModel)) return false;

        if (!ulong.TryParse((timeoutText ?? string.Empty).Trim(), out var timeoutMs)
            || timeoutMs < 1_000) return false;
        if (timeoutMs > 300_000) return false;

        ulong? contextWindowTokens = null;
        var trimmedContextWindow = (contextWindowText ?? string.Empty).Trim();
        if (!string.IsNullOrWhiteSpace(trimmedContextWindow))
        {
            if (!ulong.TryParse(trimmedContextWindow, out var parsedContextWindow)) return false;
            if (parsedContextWindow > 2_000_000) return false;
            contextWindowTokens = parsedContextWindow;
        }

        // Round-trip the actual provider type so ollama (and any future type)
        // is preserved on save instead of being silently rewritten to openai.
        // (#3131) Unknown values fall back to openai for safety.
        var normalized = (providerType ?? "openai").ToLowerInvariant();
        var ptype = normalized.Contains("anthropic") ? "anthropic"
            : normalized.Contains("ollama") ? "ollama"
            : "openai";
        config = new ProviderConfig(
            trimmedApiKey,
            trimmedBaseUrl,
            trimmedModel,
            timeoutMs,
            contextWindowTokens,
            existingMaxOutputTokens,
            ptype,
            (name ?? string.Empty).Trim());
        return true;
    }

    private void SaveCurrentProviderFields()
    {
        if (_activeProviderIndex < 0 || _activeProviderIndex >= _providers.Count) return;
        // Only persist when the current fields are valid. Previously an illegal
        // value typed before switching providers was written into _providers and
        // survived into the saved settings even if the active provider validated.
        // (#2781)
        if (TryBuildProviderConfig(
                ApiKeyBox.Password,
                BaseUrlBox.Text,
                ModelBox.Text,
                TimeoutBox.Text,
                ContextWindowBox.Text,
                _providers[_activeProviderIndex].MaxOutputTokens,
                SelectedProviderType(ProviderTypeBox.SelectedIndex),
                ProviderNameBox.Text,
                out var cfg))
        {
            _providers[_activeProviderIndex] = cfg!;
        }
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

            // ── 1. Provider validation (always runs; controls hold the current
            //        values, so an unchanged/legal config validates cleanly — #2781) ──
            bool providerValid = true;
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
            // #2823: Use TryBuildProviderConfig instead of duplicating its validation/construction logic.
            // This ensures validation and construction never drift apart.
            if (!TryBuildProviderConfig(
                    ApiKeyBox.Password,
                    BaseUrlBox.Text,
                    ModelBox.Text,
                    TimeoutBox.Text,
                    ContextWindowBox.Text,
                    _providers[_activeProviderIndex].MaxOutputTokens,
                    SelectedProviderType(ProviderTypeBox.SelectedIndex),
                    ProviderNameBox.Text,
                    out var cfg))
            {
                // Should not reach here — validation above already checked all fields.
                // Fallback: keep the current config unchanged.
                ErrorInfoBar.Message = "提供商构建失败（内部错误），当前配置未保存。";
                ErrorInfoBar.IsOpen = true;
                args.Cancel = true;
                return;
            }

            _providers[_activeProviderIndex] = cfg!;

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

            // Persist theme preference client-side and expose it so MainWindow
            // can apply it immediately after the dialog closes.
            ThemeMode = ThemeDark.IsChecked == true
                ? ElementTheme.Dark
                : ThemeLight.IsChecked == true
                    ? ElementTheme.Light
                    : ElementTheme.Default;
            ThemePreferences.Save(ThemeMode);
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

    /// <summary>
    /// Filters settings cards by search keyword (#3069).
    /// Cards whose header text or child labels contain the keyword remain visible;
    /// others are collapsed. An empty search restores all cards.
    /// </summary>
    private void OnSettingsSearchTextChanged(object sender, TextChangedEventArgs e)
    {
        var keyword = SettingsSearchBox.Text?.Trim() ?? string.Empty;
        // Collect searchable text from each card: the header TextBlock +
        // all child TextBlock/TextBox headers inside the card.
        foreach (var child in Panel.Children)
        {
            if (child is not Border card) continue;
            if (string.IsNullOrEmpty(keyword))
            {
                card.Visibility = Visibility.Visible;
                continue;
            }

            var searchText = CollectSearchText(card);
            card.Visibility = searchText.Contains(keyword, StringComparison.OrdinalIgnoreCase)
                ? Visibility.Visible
                : Visibility.Collapsed;
        }
    }

    /// <summary>
    /// Recursively collects visible text from TextBlock.Text,
    /// TextBox.Header, PasswordBox.Header, and ComboBox.Header
    /// within a UI element subtree.
    /// </summary>
    private static string CollectSearchText(DependencyObject root)
    {
        var sb = new System.Text.StringBuilder();
        CollectSearchTextRecursive(root, sb);
        return sb.ToString();
    }

    private static void CollectSearchTextRecursive(DependencyObject element, System.Text.StringBuilder sb)
    {
        if (element is TextBlock tb && !string.IsNullOrEmpty(tb.Text))
        {
            sb.Append(' ').Append(tb.Text);
        }
        if (element is TextBox tbx && !string.IsNullOrEmpty(tbx.Header?.ToString()))
        {
            sb.Append(' ').Append(tbx.Header.ToString());
        }
        if (element is PasswordBox pwb && !string.IsNullOrEmpty(pwb.Header?.ToString()))
        {
            sb.Append(' ').Append(pwb.Header.ToString());
        }
        if (element is ComboBox cb && !string.IsNullOrEmpty(cb.Header?.ToString()))
        {
            sb.Append(' ').Append(cb.Header.ToString());
        }

        var count = VisualTreeHelper.GetChildrenCount(element);
        for (var i = 0; i < count; i++)
        {
            var child = VisualTreeHelper.GetChild(element, i);
            CollectSearchTextRecursive(child, sb);
        }
    }
}
