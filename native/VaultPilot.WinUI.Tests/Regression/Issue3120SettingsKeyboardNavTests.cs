using Xunit;
using System.Text.RegularExpressions;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression tests for #3120 — WinUI Settings dialog keyboard navigation.
///
/// The feature adds keyboard accessibility to the settings dialog (#3101 only
/// shipped text filtering, no keyboard nav):
///   - Ctrl+F / Cmd+F  → re-focus search box
///   - Escape          → clear search text, or close the dialog if already empty
///   - ArrowUp/Down    → move keyboard focus between visible settings cards
///   - Enter/↓ on search box → drop into first card
///   - j/k (Vim)       → ArrowDown / ArrowUp (only when not in a text input)
///
/// These tests are structural — they assert that:
///   1. SettingsDialog.xaml wires KeyDown on SettingsSearchBox to OnSearchBoxKeyDown
///   2. SettingsDialog.xaml.cs defines every keyboard-related handler/method
///   3. The PreviewKeyDown handler is registered in WireUpButtons()
///
/// This catches the #2750-class bug where XAML references a handler that doesn't
/// exist in the code-behind (XAML compiler fails to catch this in some configs).
/// Actual runtime keyboard behaviour must be smoke-tested manually on Windows.
/// </summary>
public class Issue3120SettingsKeyboardNavTests
{
    private const string DialogXamlRelativePath =
        "VaultPilot.WinUI/Views/SettingsDialog.xaml";
    private const string DialogCsRelativePath =
        "VaultPilot.WinUI/Views/SettingsDialog.xaml.cs";

    /// <summary>
    /// The XAML must wire KeyDown on SettingsSearchBox to OnSearchBoxKeyDown.
    /// Without this binding, Enter/Escape on the search box never reaches the
    /// new handler and the keyboard contract silently fails.
    /// </summary>
    [Fact]
    public void Regression_3120_XamlBindsSearchBoxKeyDownHandler()
    {
        var xaml = ReadSourceFile(DialogXamlRelativePath);
        Assert.NotNull(xaml);

        // Look for the SettingsSearchBox element...
        var searchBoxMatch = Regex.Match(xaml,
            @"<TextBox[^>]*x:Name=""SettingsSearchBox""[^>]*>",
            RegexOptions.Singleline);
        Assert.True(searchBoxMatch.Success,
            "SettingsSearchBox TextBox element must exist in SettingsDialog.xaml");

        var searchBoxElement = searchBoxMatch.Value;
        Assert.Contains("KeyDown=\"OnSearchBoxKeyDown\"", searchBoxElement);
        Assert.Contains("TextChanged=\"OnSettingsSearchTextChanged\"", searchBoxElement);
    }

    /// <summary>
    /// The code-behind must define every handler/method referenced by the
    /// keyboard navigation contract. If any is missing or renamed, the XAML
    /// binding breaks at runtime.
    /// </summary>
    [Theory]
    [InlineData("OnSearchBoxKeyDown")]
    [InlineData("OnDialogPreviewKeyDown")]
    [InlineData("MoveCardFocus")]
    [InlineData("GetVisibleSettingsCards")]
    [InlineData("ClearSearchOrClose")]
    [InlineData("FocusSearchBox")]
    [InlineData("IsFocusInsideTextInput")]
    [InlineData("FindFirstFocusableControl")]
    public void Regression_3120_CodeBehindDefinesAllKeyboardMethods(string methodName)
    {
        var cs = ReadSourceFile(DialogCsRelativePath);
        Assert.NotNull(cs);

        // Each method must appear as a definition, not just a reference.
        var methodDefPattern = $@"(private\s+static\s+|private\s+)([A-Za-z0-9_<>\?\[\]]+\s+)?{methodName}\s*\(";
        var match = Regex.Match(cs, methodDefPattern);
        Assert.True(match.Success,
            $"SettingsDialog.xaml.cs must define method '{methodName}' for #3120 keyboard nav. " +
            $"Pattern: {methodDefPattern}");
    }

    /// <summary>
    /// The PreviewKeyDown handler must be registered on the dialog in
    /// WireUpButtons(). Without this registration, arrow keys / Ctrl+F / Esc
    /// won't trigger the keyboard nav code paths.
    /// </summary>
    [Fact]
    public void Regression_3120_PreviewKeyDownIsRegisteredInConstructor()
    {
        var cs = ReadSourceFile(DialogCsRelativePath);
        Assert.NotNull(cs);

        // The registration line: PreviewKeyDown += OnDialogPreviewKeyDown;
        Assert.Contains("PreviewKeyDown += OnDialogPreviewKeyDown", cs);

        // And the handler must exist.
        Assert.Matches(@"void\s+OnDialogPreviewKeyDown\s*\(", cs);
    }

    /// <summary>
    /// The keyboard focus state field must exist — without it, MoveCardFocus
    /// can't track which card is currently focused and wrapping breaks.
    /// </summary>
    [Fact]
    public void Regression_3120_FocusIndexFieldExists()
    {
        var cs = ReadSourceFile(DialogCsRelativePath);
        Assert.NotNull(cs);

        Assert.Contains("_keyboardFocusCardIndex", cs);
        Assert.Matches(@"private\s+int\s+_keyboardFocusCardIndex\s*=\s*-1", cs);
    }

    /// <summary>
    /// Ctrl+F binding must invoke the search box focus — Obsidian 1.13.0-style
    /// "re-focus search from anywhere". Verify the key check + the focus call
    /// both exist (this guards against accidental removal during refactoring).
    /// </summary>
    [Fact]
    public void Regression_3120_CtrlFHandlerCallsFocusSearchBox()
    {
        var cs = ReadSourceFile(DialogCsRelativePath);
        Assert.NotNull(cs);

        // Find the OnDialogPreviewKeyDown method body.
        var handlerStart = cs.IndexOf("void OnDialogPreviewKeyDown(", StringComparison.Ordinal);
        Assert.True(handlerStart >= 0, "OnDialogPreviewKeyDown method must exist");

        // Slice from handler start to next "private" keyword (approximate method end).
        var sliceStart = handlerStart;
        var nextPrivate = cs.IndexOf("\n    private ", sliceStart + 1, StringComparison.Ordinal);
        var handlerBody = nextPrivate > 0
            ? cs[sliceStart..nextPrivate]
            : cs[sliceStart..];

        // Body must reference VirtualKey.F + modifier check + FocusSearchBox().
        Assert.Contains("VirtualKey.F", handlerBody);
        Assert.Contains("FocusSearchBox()", handlerBody);
    }

    /// <summary>
    /// Escape must implement "clear search OR close dialog" semantics, not just
    /// one or the other. Both branches must be present in ClearSearchOrClose().
    /// </summary>
    [Fact]
    public void Regression_3120_EscapeClearsSearchOrClosesDialog()
    {
        var cs = ReadSourceFile(DialogCsRelativePath);
        Assert.NotNull(cs);

        var handlerStart = cs.IndexOf("void ClearSearchOrClose(", StringComparison.Ordinal);
        Assert.True(handlerStart >= 0, "ClearSearchOrClose method must exist");

        var nextPrivate = cs.IndexOf("\n    private ", handlerStart + 1, StringComparison.Ordinal);
        var methodBody = nextPrivate > 0
            ? cs[handlerStart..nextPrivate]
            : cs[handlerStart..];

        // Both branches must exist.
        Assert.Contains("SettingsSearchBox.Text = string.Empty", methodBody);
        Assert.Contains("Hide()", methodBody);
    }

    private static string? ReadSourceFile(string relativePath)
    {
        // Walk up from test bin directory to find the repo root, then resolve.
        var candidates = new[]
        {
            // dotnet test from repo root: <repo>/<relativePath>
            Path.Combine(AppContext.BaseDirectory, "..", "..", "..", "..", "..", relativePath),
            // dotnet test from project: <repo>/<relativePath> via one fewer ..
            Path.Combine(AppContext.BaseDirectory, "..", "..", "..", "..", relativePath),
            // CI may run from a different layout.
            Path.Combine(AppContext.BaseDirectory, relativePath),
        };

        foreach (var candidate in candidates)
        {
            if (File.Exists(candidate))
            {
                return File.ReadAllText(candidate);
            }
        }
        return null;
    }
}
