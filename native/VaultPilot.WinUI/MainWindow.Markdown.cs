using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System.Collections.Generic;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Text.RegularExpressions;
using VaultPilot.WinUI.Utils;

namespace VaultPilot.WinUI;

/// <summary>
/// Markdown rendering methods extracted from MainWindow for SRP compliance.
/// Also handles [[wikilink]] and auto-detected note references (#2035).
/// </summary>
public sealed partial class MainWindow : Window
{
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
                ? GetThemeBrush("TextOnAccentFillColorPrimaryBrush")
                : GetThemeBrush("TextFillColorPrimaryBrush")
        };
    }

    internal FrameworkElement CreateMarkdownContent(string markdown)
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
        AutomationProperties.SetName(copyButton, "复制 Markdown");
        copyButton.Click += (_, _) => CopyTextToClipboard(markdown);
        stack.Children.Add(copyButton);

        // Collect all image paths across blocks for lightbox navigation (#3693).
        var allImagePaths = CollectMarkdownImages(markdown);

        foreach (var block in ParseMarkdownBlocks(markdown))
        {
            if (block.IsCode)
            {
                stack.Children.Add(CreateCodeBlock(block.Text, block.Language));
                continue;
            }


            if (block.IsTable)
            {
                stack.Children.Add(CreateMarkdownTable(block.Text));
                continue;
            }
            foreach (var element in CreateMarkdownTextElements(block.Text, allImagePaths))
            {
                stack.Children.Add(element);
            }
        }

        return stack;
    }

    private IEnumerable<FrameworkElement> CreateMarkdownTextElements(
        string text, IReadOnlyList<string> allImagePaths)
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

            // Image line (#3693): render clickable thumbnail opening lightbox
            // #3749: reuse compiled static MarkdownImagePattern instead of per-line alloc
            var imgMatch = MarkdownImagePattern.Match(line);
            if (imgMatch.Success)
            {
                yield return CreateMarkdownImage(
                    imgMatch.Groups["url"].Value,
                    imgMatch.Groups["alt"].Value,
                    allImagePaths);
                continue;
            }

            var textBlock = new TextBlock
            {
                TextWrapping = TextWrapping.Wrap,
                IsTextSelectionEnabled = true,
                Foreground = GetThemeBrush("TextFillColorPrimaryBrush")
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
                textBlock.Foreground = GetThemeBrush("TextFillColorSecondaryBrush");
                textBlock.Padding = new Thickness(12, 4, 4, 4);
                ApplyInlineMarkdown(textBlock, quoteText.Trim());

                var border = new Border
                {
                    BorderBrush = GetThemeBrush("ControlStrokeColorDefaultBrush"),
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

    private void ApplyInlineMarkdown(TextBlock textBlock, string text)
    {
        textBlock.Inlines.Clear();
        AppendInlineMarkdown(textBlock.Inlines, text);
    }

    private void AppendInlineMarkdown(InlineCollection inlines, string text)
    {
        if (string.IsNullOrEmpty(text))
        {
            return;
        }

        var index = 0;
        while (index < text.Length)
        {
            // Check for inline code (`code`) — must be before [[wikilink]] to
            // prevent wikilink parsing inside code backticks (#2589).
            if (text[index] == '`')
            {
                var closeIndex = text.IndexOf('`', index + 1);
                if (closeIndex > index)
                {
                    var span = new Span
                    {
                        FontFamily = new FontFamily("Cascadia Code"),
                        Foreground = GetThemeBrush("CodeInlineForegroundBrush")
                    };
                    span.Inlines.Add(new Run { Text = text[(index + 1)..closeIndex] });
                    inlines.Add(span);
                    index = closeIndex + 1;
                    continue;
                }
            }

            // Check for [[wikilink]] — must be before [link] to avoid double-bracket confusion
            if (text[index] == '[' && index + 1 < text.Length && text[index + 1] == '[')
            {
                var closeBracket = text.IndexOf("]]", index + 2, StringComparison.Ordinal);
                if (closeBracket > index + 1)
                {
                    var wikiTitle = text[(index + 2)..closeBracket].Trim();
                    if (!string.IsNullOrEmpty(wikiTitle))
                    {
                        var hyperlink = new Hyperlink
                        {
                            UnderlineStyle = UnderlineStyle.Single,
                            Foreground = GetThemeBrush("AccentTextFillColorPrimaryBrush")
                        };
                        hyperlink.Inlines.Add(new Run { Text = $"📄 {wikiTitle}" });
                        var capturedTitle = wikiTitle;
                        hyperlink.Click += async (_, _) => await NavigateToNoteFromTitleAsync(capturedTitle);
                        AutomationProperties.SetName(hyperlink, $"打开笔记: {wikiTitle}");
                        inlines.Add(hyperlink);
                        index = closeBracket + 2;
                        continue;
                    }
                }
            }

            // Check for markdown link [text](url)
            if (text[index] == '[')
            {
                var closeBracket = text.IndexOf(']', index + 1);
                if (closeBracket > index + 1
                    && closeBracket + 1 < text.Length
                    && text[closeBracket + 1] == '(')
                {
                    var closeParen = text.IndexOf(')', closeBracket + 2);
                    if (closeParen > closeBracket + 1)
                    {
                        var linkText = text[(index + 1)..closeBracket];
                        var linkUrl = text[(closeBracket + 2)..closeParen];
                        if (Uri.TryCreate(linkUrl, UriKind.Absolute, out var uri)
                            && uri.Scheme is "http" or "https")
                        {
                            var hyperlink = new Hyperlink
                            {
                                NavigateUri = uri,
                                UnderlineStyle = UnderlineStyle.Single
                            };
                            hyperlink.Click += Hyperlink_Click;
                            AppendInlineMarkdown(hyperlink.Inlines, linkText);
                            inlines.Add(hyperlink);
                            index = closeParen + 1;
                            continue;
                        }
                    }
                }
            }

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
            if (nextIndex <= index)
            {
                nextIndex = index + 1; // forward-progress guarantee: emit the unmatched char as plain text
            }
            AppendNoteRefText(inlines, text[index..nextIndex]);
            index = nextIndex;
        }
    }

    /// <summary>
    /// Emits a plain text segment, splitting it into note reference hyperlinks
    /// when the note title map is available and note titles are detected.
    /// </summary>
    private void AppendNoteRefText(InlineCollection inlines, string text)
    {
        if (string.IsNullOrEmpty(text))
            return;

        // #3581: Very short text can't match any meaningful note title.
        if (text.Length < 4)
        {
            inlines.Add(new Run { Text = text });
            return;
        }

        // Only check for note refs if the title map has been loaded
        var titleMap = _noteTitleMap;
        if (titleMap is null || titleMap.Count == 0)
        {
            inlines.Add(new Run { Text = text });
            return;
        }

        var refs = NoteRefs.FindNoteReferences(text, titleMap);
        if (refs.Count == 0)
        {
            inlines.Add(new Run { Text = text });
            return;
        }

        var segments = NoteRefs.SplitLineByNoteRefs(text, refs);
        foreach (var seg in segments)
        {
            if (seg.IsNoteRef && seg.Title is not null)
            {
                var hyperlink = new Hyperlink
                {
                    UnderlineStyle = UnderlineStyle.Single,
                    Foreground = GetThemeBrush("AccentTextFillColorPrimaryBrush")
                };
                hyperlink.Inlines.Add(new Run { Text = $"📄 {seg.Title}" });
                var capturedTitle = seg.Title;
                hyperlink.Click += async (_, _) => await NavigateToNoteFromTitleAsync(capturedTitle);
                AutomationProperties.SetName(hyperlink, $"打开笔记: {seg.Title}");
                inlines.Add(hyperlink);
            }
            else
            {
                inlines.Add(new Run { Text = seg.Text });
            }
        }
    }

    private static readonly string[] _inlineMarkers = { "**", "*", "`", "[" };

    private static int FindNextInlineMarker(string text, int startIndex)
    {
        var nextIndex = text.Length;
        foreach (var marker in _inlineMarkers)
        {
            var index = text.IndexOf(marker, startIndex, StringComparison.Ordinal);
            if (index >= 0 && index < nextIndex)
            {
                nextIndex = index;
            }
        }

        return nextIndex;
    }

    private static async void Hyperlink_Click(Hyperlink sender, HyperlinkClickEventArgs args)
    {
        try
        {
            if (sender.NavigateUri != null
                && sender.NavigateUri.Scheme is "http" or "https")
            {
                await Windows.System.Launcher.LaunchUriAsync(sender.NavigateUri);
            }
        }
        catch (Exception error)
        {
            System.Diagnostics.Debug.WriteLine($"[Hyperlink_Click] Error: {error}");
        }
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
        AutomationProperties.SetName(copyButton, "复制代码");
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
            FontFamily = new FontFamily("Cascadia Code"),
            Foreground = GetThemeBrush("CodeBlockForegroundBrush")
        };
        // Horizontal-scroll wrapper so very long lines (long URLs, minified
        // code, base64) that TextWrapping cannot break don't overflow and get
        // clipped by the message bubble.
        var codeScroll = new ScrollViewer
        {
            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
            VerticalScrollBarVisibility = ScrollBarVisibility.Disabled,
            HorizontalScrollMode = ScrollMode.Enabled,
            VerticalScrollMode = ScrollMode.Disabled,
            Content = codeText
        };

        return new Border
        {
            CornerRadius = new CornerRadius(12),
            Padding = new Thickness(12),
            Background = GetThemeBrush("CodeBlockBackgroundBrush"),
            HorizontalAlignment = HorizontalAlignment.Stretch,
            Child = new StackPanel
            {
                Spacing = 8,
                Children =
                {
                    header,
                    codeScroll
                }
            }
        };
    }

    private FrameworkElement CreateMarkdownTable(string tableText)
    {
        var lines = tableText.Split('\n')
            .Select(l => l.Trim())
            .Where(l => l.StartsWith("|") && l.EndsWith("|") && l.Length > 2)
            .ToList();

        if (lines.Count < 2)
        {
            return new TextBlock
            {
                Text = tableText,
                TextWrapping = TextWrapping.Wrap,
                IsTextSelectionEnabled = true,
                Foreground = GetThemeBrush("TextFillColorPrimaryBrush")
            };
        }

        // Remove separator line (|---|---|)
        if (lines[1].Contains("---"))
        {
            lines.RemoveAt(1);
        }

        // Parse cells from each row
        var rows = new List<string[]>();
        foreach (var line in lines)
        {
            var cells = line.Split('|')
                .Skip(1)       // Skip empty segment before first |
                .SkipLast(1)   // Skip empty segment after last |
                .Select(c => c.Trim())
                .ToArray();
            rows.Add(cells);
        }

        if (rows.Count == 0)
        {
            return new TextBlock
            {
                Text = tableText,
                TextWrapping = TextWrapping.Wrap,
                IsTextSelectionEnabled = true,
                Foreground = GetThemeBrush("TextFillColorPrimaryBrush")
            };
        }

        var colCount = rows.Max(r => r.Length);
        var grid = new Grid();

        for (var c = 0; c < colCount; c++)
        {
            grid.ColumnDefinitions.Add(new ColumnDefinition
            {
                Width = new GridLength(1, GridUnitType.Auto)
            });
        }

        for (var r = 0; r < rows.Count; r++)
        {
            grid.RowDefinitions.Add(new RowDefinition
            {
                Height = new GridLength(1, GridUnitType.Auto)
            });

            for (var c = 0; c < colCount; c++)
            {
                var cellText = c < rows[r].Length ? rows[r][c] : "";
                var cellBlock = new TextBlock
                {
                    TextWrapping = TextWrapping.Wrap,
                    IsTextSelectionEnabled = true,
                    Padding = new Thickness(8, 6, 8, 6),
                    Foreground = GetThemeBrush("TextFillColorPrimaryBrush")
                };
                ApplyInlineMarkdown(cellBlock, cellText);

                // Bold the header row
                if (r == 0)
                {
                    cellBlock.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
                }

                var cellBorder = new Border
                {
                    BorderBrush = GetThemeBrush("ControlStrokeColorDefaultBrush"),
                    BorderThickness = new Thickness(
                        c == 0 ? 0 : 0.5,
                        r == 0 ? 0 : 0.5,
                        0.5,
                        0.5),
                    Child = cellBlock
                };

                Grid.SetRow(cellBorder, r);
                Grid.SetColumn(cellBorder, c);
                grid.Children.Add(cellBorder);
            }
        }

        return new Border
        {
            BorderBrush = GetThemeBrush("ControlStrokeColorDefaultBrush"),
            BorderThickness = new Thickness(0.5),
            CornerRadius = new CornerRadius(4),
            Margin = new Thickness(0, 4, 0, 4),
            Child = grid
        };
    }

    private IEnumerable<(bool IsCode, bool IsTable, string Text, string? Language)> ParseMarkdownBlocks(string markdown)
    {
        var normalized = markdown.Replace("\r\n", "\n");
        var parts = normalized.Split("```");
        for (var i = 0; i < parts.Length; i++)
        {
            if (i % 2 == 0)
            {
                if (!string.IsNullOrWhiteSpace(parts[i]))
                {
                    foreach (var segment in SplitTextAndTables(parts[i].Trim()))
                    {
                        yield return segment;
                    }
                }

                continue;
            }

            var block = parts[i];
            var firstNewline = block.IndexOf('\n');
            if (firstNewline < 0)
            {
                yield return (true, false, block.Trim(), null);
                continue;
            }

            var language = block[..firstNewline].Trim();
            var code = block[(firstNewline + 1)..].TrimEnd();
            yield return (true, false, code, string.IsNullOrWhiteSpace(language) ? null : language);
        }
    }

    private static IEnumerable<(bool IsCode, bool IsTable, string Text, string? Language)> SplitTextAndTables(string text)
    {
        var lines = text.Split('\n');
        var currentTextLines = new List<string>();
        var tableLines = new List<string>();
        var inTable = false;

        for (var i = 0; i < lines.Length; i++)
        {
            var line = lines[i].Trim();
            var isTableRow = line.StartsWith("|") && line.EndsWith("|") && line.Length > 2;

            if (isTableRow && !inTable)
            {
                // Check if the next line is a table separator (|---|---|)
                if (i + 1 < lines.Length)
                {
                    var nextLine = lines[i + 1].Trim();
                    if (nextLine.StartsWith("|") && nextLine.Contains("---"))
                    {
                        // Flush any pending text lines
                        if (currentTextLines.Count > 0)
                        {
                            var textBlock = string.Join("\n", currentTextLines).Trim();
                            if (!string.IsNullOrWhiteSpace(textBlock))
                            {
                                yield return (false, false, textBlock, null);
                            }
                            currentTextLines.Clear();
                        }

                        inTable = true;
                        tableLines.Add(lines[i]);
                        continue;
                    }
                }

                // Not a recognized table, treat as regular text
                currentTextLines.Add(lines[i]);
            }
            else if (isTableRow && inTable)
            {
                tableLines.Add(lines[i]);
            }
            else
            {
                if (inTable)
                {
                    yield return (false, true, string.Join("\n", tableLines).Trim(), null);
                    tableLines.Clear();
                    inTable = false;
                }

                currentTextLines.Add(lines[i]);
            }
        }

        // Flush remaining table
        if (inTable && tableLines.Count > 0)
        {
            yield return (false, true, string.Join("\n", tableLines).Trim(), null);
        }

        // Flush remaining text
        if (currentTextLines.Count > 0)
        {
            var remaining = string.Join("\n", currentTextLines).Trim();
            if (!string.IsNullOrWhiteSpace(remaining))
            {
                yield return (false, false, remaining, null);
            }
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

    [GeneratedRegex(@"\[[^\]]+\]\(https?://[^)]+\)", RegexOptions.Compiled)]
    private static partial Regex MarkdownLinkPattern();

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

        // Detect markdown tables and links
        if (text.Contains("|---", StringComparison.Ordinal)
            || MarkdownLinkPattern().IsMatch(text))
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

    // ── Image lightbox integration (#3693) ──────────────────────────

    private static readonly Regex MarkdownImagePattern = new(
        @"^!\[(?<alt>[^\]]*)\]\((?<url>[^)]+)\)$",
        RegexOptions.Compiled);

    /// <summary>
    /// Scan all markdown blocks and collect image URLs for lightbox
    /// cross-image navigation (#3693).
    /// </summary>
    private static List<string> CollectMarkdownImages(string markdown)
    {
        var paths = new List<string>();
        var normalized = markdown.Replace("\r\n", "\n");
        var parts = normalized.Split("```");
        for (var i = 0; i < parts.Length; i++)
        {
            if (i % 2 != 0) continue; // skip code blocks
            foreach (var line in parts[i].Split('\n'))
            {
                var trimmed = line.Trim();
                var m = MarkdownImagePattern.Match(trimmed);
                if (m.Success)
                    paths.Add(m.Groups["url"].Value);
            }
        }
        return paths;
    }

    /// <summary>
    /// Create a clickable image thumbnail for a markdown image line.
    /// Clicking opens the full image lightbox (#3693).
    /// </summary>
    private FrameworkElement CreateMarkdownImage(
        string imagePath, string altText, IReadOnlyList<string> allImagePaths)
    {
        var image = new Image
        {
            Stretch = Stretch.Uniform,
            MaxWidth = 400,
            MaxHeight = 300,
            HorizontalAlignment = HorizontalAlignment.Left,
            Source = TryCreateImageSource(imagePath),
        };

        if (!string.IsNullOrWhiteSpace(altText))
            AutomationProperties.SetName(image, altText);
        else
            AutomationProperties.SetName(image, "Image");

        // ── Click → open lightbox (Tapped for touch-scroll safety, #3748) ──
        var capturedPath = imagePath;
        var capturedPaths = allImagePaths;
        image.Tapped += async (sender, args) =>
        {
            var idx = -1;
            for (var i = 0; i < capturedPaths.Count; i++)
            {
                if (string.Equals(capturedPaths[i], capturedPath, StringComparison.Ordinal))
                {
                    idx = i;
                    break;
                }
            }
            if (idx < 0) idx = 0;
            await ShowImageLightboxAsync(
                capturedPaths, idx,
                LoadLightboxImageAsync, removable: false);
        };

        return new Border
        {
            Child = image,
            Margin = new Thickness(0, 4, 0, 4),
            HorizontalAlignment = HorizontalAlignment.Left,
        };
    }

    /// <summary>
    /// Try to create an ImageSource from a path — supports HTTP(S) URLs
    /// and local files.
    /// </summary>
    private static ImageSource? TryCreateImageSource(string path)
    {
        try
        {
            if (Uri.TryCreate(path, UriKind.Absolute, out var uri)
                && (uri.Scheme == Uri.UriSchemeHttp || uri.Scheme == Uri.UriSchemeHttps))
                return new BitmapImage(uri);

            if (System.IO.File.Exists(path))
                return new BitmapImage(new Uri(path, UriKind.Absolute));

            return null;
        }
        catch { return null; }
    }

    /// <summary>
    /// Lightbox image loader for markdown images (#3693).
    /// #3749: skip backend round-trip for HTTP(S) URLs (always fails).
    /// Tries the backend preview API first, then falls back to HTTP(S) URL.
    /// </summary>
    private async Task<BitmapImage?> LoadLightboxImageAsync(string path)
    {
        try
        {
            // #3749: HTTP(S) URLs don't need backend — go straight to fallback
            if (Uri.TryCreate(path, UriKind.Absolute, out var uri)
                && (uri.Scheme == Uri.UriSchemeHttp || uri.Scheme == Uri.UriSchemeHttps))
                return new BitmapImage(uri);

            if (_backendClient is not null)
            {
                using var cts = new System.Threading.CancellationTokenSource(
                    TimeSpan.FromSeconds(30));
                // #3747: readFileAsDataUrl supports paths outside vault
                var dataUrl = await _backendClient.SendAsync<string>(
                    "readFileAsDataUrl", new { path }, cts.Token);
                if (!string.IsNullOrWhiteSpace(dataUrl))
                {
                    var bytes = DecodeDataUrl(dataUrl);
                    using var stream = new Windows.Storage.Streams.InMemoryRandomAccessStream();
                    await stream.WriteAsync(bytes.AsBuffer());
                    stream.Seek(0);
                    var bitmap = new BitmapImage();
                    await bitmap.SetSourceAsync(stream);
                    return bitmap;
                }
            }
        }
        catch { /* fall through to URL fallback */ }

        // Fallback: HTTP(S) URL (only reached if backend fails AND path is HTTP)
        if (Uri.TryCreate(path, UriKind.Absolute, out var uri2)
            && (uri2.Scheme == Uri.UriSchemeHttp || uri2.Scheme == Uri.UriSchemeHttps))
            return new BitmapImage(uri2);

        return null;
    }
}
