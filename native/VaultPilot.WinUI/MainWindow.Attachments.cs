using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System.Runtime.InteropServices.WindowsRuntime;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using Windows.Storage.Streams;

namespace VaultPilot.WinUI;

/// <summary>
/// Attachment management methods extracted from MainWindow for SRP compliance.
/// </summary>
public sealed partial class MainWindow : Window
{
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
        // File icon
        var icon = new FontIcon
        {
            Glyph = "\uE7C3", // Photo icon
            FontSize = 14,
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(0, 0, 4, 0)
        };

        // Filename text (truncated)
        var nameText = new TextBlock
        {
            Text = attachment.Name,
            MaxWidth = 120,
            TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center,
            FontSize = 12
        };

        // Remove button (X)
        var removeButton = new Button
        {
            Content = "\uE711", // Cancel icon
            Padding = new Thickness(2),
            MinWidth = 0,
            MinHeight = 0,
            FontSize = 10,
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(4, 0, 0, 0),
            Background = _transparentBrush
        };
        AutomationProperties.SetName(removeButton, $"移除附件 {attachment.Name}");
        removeButton.Click += (_, _) =>
        {
            _attachments.RemoveAll(item => item.Path == attachment.Path);
            RefreshAttachments();
            UpdateStatusBar("info", "图片已移除", $"当前还剩 {_attachments.Count} 张图片。");
        };

        var chip = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 2,
            Padding = new Thickness(8, 4, 4, 4),
            VerticalAlignment = VerticalAlignment.Center
        };
        chip.Children.Add(icon);
        chip.Children.Add(nameText);
        chip.Children.Add(removeButton);

        var chipBorder = new Border
        {
            MinWidth = 120,
            MaxWidth = 200,
            CornerRadius = new CornerRadius(4),
            Background = GetThemeBrush("CardBackgroundFillColorSecondaryBrush"),
            BorderBrush = GetThemeBrush("AttachmentBorderBrush"),
            BorderThickness = new Thickness(1),
            Margin = new Thickness(0, 0, 2, 0),
            Child = chip
        };

        ToolTipService.SetToolTip(chipBorder, $"{attachment.Name}\n单击预览");
        chipBorder.Tapped += async (_, _) =>
        {
            try
            {
                await ShowImagePreviewDialogAsync(attachment, removable: true);
            }
            catch (Exception ex)
            {
                System.Diagnostics.Debug.WriteLine($"Image preview failed: {ex.Message}");
            }
        };

        return chipBorder;
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
        image.Tapped += async (_, _) =>
        {
            try
            {
                await ShowImagePreviewDialogAsync(attachment);
            }
            catch (Exception ex)
            {
                System.Diagnostics.Debug.WriteLine($"Image preview failed: {ex.Message}");
            }
        };

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
            Background = GetThemeBrush("CardBackgroundFillColorSecondaryBrush"),
            BorderBrush = GetThemeBrush("CardStrokeColorDefaultBrush"),
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
        using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
        var dataUrl = await _backendClient.SendAsync<string>("readImagePreview", new { path }, cts.Token);
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

        var fileName = $"clipboard-{DateTimeOffset.Now:yyyyMMdd-HHmmssfff}-{Path.GetFileNameWithoutExtension(Path.GetRandomFileName())}.png";
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
}
