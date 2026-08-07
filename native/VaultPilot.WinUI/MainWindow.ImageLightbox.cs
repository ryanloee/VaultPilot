using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Windows.Foundation;
using Windows.System;

namespace VaultPilot.WinUI;

/// <summary>
/// Image Lightbox — full-screen image viewer with zoom, pan, and keyboard
/// navigation across a list of images (issues #3469, #3790, mirroring the mobile
/// Lightbox.tsx behaviour). Built programmatically to match the code-built
/// UI pattern used by <c>MainWindow.Attachments.cs</c>.
/// </summary>
public sealed partial class MainWindow
{
    // ── Lightbox state ──────────────────────────────────────────────
    private IReadOnlyList<string> _lightboxPaths = Array.Empty<string>();
    private int _lightboxIndex;
    private Func<string, Task<BitmapImage?>>? _lightboxImageLoader;
    private ContentDialog? _lightboxDialog;
    private Image? _lightboxImage;
    private ScaleTransform? _lightboxScale;
    private TranslateTransform? _lightboxTranslate;
    private TextBlock? _lightboxZoomLabel;
    private TextBlock? _lightboxIndexLabel;
    private TextBlock? _lightboxFileNameLabel;
    private Button? _lightboxPrevBtn;
    private Button? _lightboxNextBtn;

    private const double LightboxMinZoom = 1.0;
    private const double LightboxMaxZoom = 5.0;
    private const double LightboxZoomStep = 0.5;

    // Pan tracking
    private bool _lightboxPanning;
    private Point _lightboxPanLast;

    // Swipe-to-dismiss tracking (#3751)
    private double _lightboxSwipeY;
    private bool _lightboxSwipeActive;

    /// <summary>
    /// Show a full-screen image lightbox.
    /// </summary>
    /// <param name="paths">Ordered list of image paths to navigate.</param>
    /// <param name="startIndex">Index into <paramref name="paths"/> to show first.</param>
    /// <param name="imageLoader">Loads a <see cref="BitmapImage"/> for a given path (async).</param>
    /// <param name="removable">If true, a "移除" affordance is offered.</param>
    private async Task ShowImageLightboxAsync(
        IReadOnlyList<string> paths,
        int startIndex,
        Func<string, Task<BitmapImage?>> imageLoader,
        bool removable = false)
    {
        if (paths.Count == 0)
        {
            return;
        }

        _lightboxPaths = paths;
        _lightboxImageLoader = imageLoader;
        _lightboxIndex = Math.Clamp(startIndex, 0, paths.Count - 1);

        var root = BuildLightboxRoot(removable);
        _lightboxDialog = new ContentDialog
        {
            XamlRoot = RootGrid.XamlRoot,
            FullSizeDesired = true,
            Content = root,
            // No buttons — we render our own close control so the dialog
            // chrome doesn't steal vertical space from the image.
            CloseButtonText = string.Empty,
        };

        // Keyboard handling at the dialog content level.
        root.KeyDown += Lightbox_OnKeyDown;
        root.PointerWheelChanged += Lightbox_OnPointerWheel;
        root.PointerPressed += Lightbox_OnPointerPressed;
        root.PointerMoved += Lightbox_OnPointerMoved;
        root.PointerReleased += Lightbox_OnPointerReleased;
        root.ManipulationMode = ManipulationModes.TranslateY | ManipulationModes.TranslateInertia;
        root.ManipulationDelta += Lightbox_OnManipulationDelta;
        root.ManipulationCompleted += Lightbox_OnManipulationCompleted;

        UpdateLightboxNavButtons();

        var dialogTask = _lightboxDialog.ShowAsync().AsTask();
        await LoadLightboxImageAsync(_lightboxIndex);
        _ = root.Focus(FocusState.Programmatic);
        await dialogTask;
    }

    private Grid BuildLightboxRoot(bool removable)
    {
        _lightboxScale = new ScaleTransform { ScaleX = 1, ScaleY = 1 };
        _lightboxTranslate = new TranslateTransform { X = 0, Y = 0 };
        var transformGroup = new TransformGroup();
        transformGroup.Children.Add(_lightboxScale);
        transformGroup.Children.Add(_lightboxTranslate);

        _lightboxImage = new Image
        {
            Stretch = Stretch.Uniform,
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
            RenderTransform = transformGroup,
            RenderTransformOrigin = new Point(0.5, 0.5),
        };

        var imageStage = new Grid
        {
            Background = new SolidColorBrush(Microsoft.UI.Colors.Black),
        };
        imageStage.Children.Add(_lightboxImage);

        // ── Top bar: index label + zoom label + zoom controls + close ──
        _lightboxIndexLabel = new TextBlock
        {
            Foreground = new SolidColorBrush(Microsoft.UI.Colors.White),
            FontSize = 13,
            VerticalAlignment = VerticalAlignment.Center,
        };
        // #3927: current image file name caption (mirrors Obsidian 1.13.4)
        _lightboxFileNameLabel = new TextBlock
        {
            Foreground = new SolidColorBrush(Microsoft.UI.Colors.White),
            FontSize = 13,
            VerticalAlignment = VerticalAlignment.Center,
            HorizontalAlignment = HorizontalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis,
            MaxLines = 1,
            Text = string.Empty,
        };
        _lightboxZoomLabel = new TextBlock
        {
            Foreground = new SolidColorBrush(Microsoft.UI.Colors.White),
            FontSize = 13,
            VerticalAlignment = VerticalAlignment.Center,
            Text = "100%",
        };

        var zoomInBtn = MakeLightboxIconButton("+", "放大", Lightbox_ZoomIn);
        var zoomOutBtn = MakeLightboxIconButton("−", "缩小", Lightbox_ZoomOut);
        var resetBtn = MakeLightboxIconButton("1:1", "100%", Lightbox_ResetZoom);
        var fitBtn = MakeLightboxIconButton("↕", "适应屏幕", Lightbox_ResetZoom);
        var closeBtn = MakeLightboxIconButton("✕", "关闭", Lightbox_Close);

        var topBar = new Grid
        {
            Height = 44,
            Padding = new Thickness(12, 0, 12, 0),
            HorizontalAlignment = HorizontalAlignment.Stretch,
            VerticalAlignment = VerticalAlignment.Top,
        };
        topBar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Auto) });
        topBar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        topBar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Auto) });
        topBar.Children.Add(_lightboxIndexLabel);
        Grid.SetColumn(_lightboxIndexLabel, 0);

        // File name caption sits in the stretchable middle column so it
        // centers between the index label and the zoom controls (#3927).
        topBar.Children.Add(_lightboxFileNameLabel);
        Grid.SetColumn(_lightboxFileNameLabel, 1);

        var controlsStack = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 8,
            HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Center,
        };
        controlsStack.Children.Add(_lightboxZoomLabel);
        controlsStack.Children.Add(zoomOutBtn);
        controlsStack.Children.Add(zoomInBtn);
        controlsStack.Children.Add(resetBtn);
        controlsStack.Children.Add(fitBtn);
        if (removable)
        {
            controlsStack.Children.Add(MakeLightboxIconButton("🗑", "移除", Lightbox_Remove));
        }
        controlsStack.Children.Add(closeBtn);
        topBar.Children.Add(controlsStack);
        Grid.SetColumn(controlsStack, 2);

        // ── Navigation chevrons (only meaningful when > 1 image) ──
        _lightboxPrevBtn = MakeLightboxIconButton("‹", "上一张", (_, _) => Lightbox_Navigate(-1));
        _lightboxNextBtn = MakeLightboxIconButton("›", "下一张", (_, _) => Lightbox_Navigate(1));
        _lightboxPrevBtn.VerticalAlignment = VerticalAlignment.Center;
        _lightboxPrevBtn.HorizontalAlignment = HorizontalAlignment.Left;
        _lightboxNextBtn.VerticalAlignment = VerticalAlignment.Center;
        _lightboxNextBtn.HorizontalAlignment = HorizontalAlignment.Right;

        var root = new Grid
        {
            Background = new SolidColorBrush(Microsoft.UI.Colors.Black),
            RequestedTheme = ElementTheme.Dark,
        };
        root.Children.Add(imageStage);
        root.Children.Add(_lightboxPrevBtn);
        root.Children.Add(_lightboxNextBtn);
        root.Children.Add(topBar);
        // Allow the root to receive keyboard focus & key events.
        root.IsTabStop = true;
        return root;
    }

    private Button MakeLightboxIconButton(string label, string tooltip, RoutedEventHandler click)
    {
        var btn = new Button
        {
            Content = new TextBlock { Text = label, FontSize = 16 },
            Width = 40,
            Height = 40,
            Padding = new Thickness(0),
            MinWidth = 0,
            MinHeight = 0,
            TabIndex = 0,
        };
        ToolTipService.SetToolTip(btn, tooltip);
        AutomationProperties.SetName(btn, tooltip);
        btn.Click += click;
        return btn;
    }

    private async Task LoadLightboxImageAsync(int index)
    {
        if (_lightboxImage is null || _lightboxImageLoader is null)
        {
            return;
        }
        if (index < 0 || index >= _lightboxPaths.Count)
        {
            return;
        }

        _lightboxImage.Opacity = 0.35;
        try
        {
            var bitmap = await _lightboxImageLoader(_lightboxPaths[index]);
            // Staleness check: if the user navigated away while we were loading,
            // discard this result (issue #3530).
            if (_lightboxIndex != index) return;
            if (bitmap is not null)
            {
                _lightboxImage.Source = bitmap;
                _lightboxImage.Opacity = 1;
            }
        }
        catch
        {
            _lightboxImage.Opacity = 0.35;
        }

        // Re-check staleness after catch — the index may have changed during the
        // exception handler (unlikely but consistent).
        if (_lightboxIndex != index) return;

        Lightbox_ResetZoom();
        UpdateLightboxNavButtons();
        if (_lightboxIndexLabel is not null)
        {
            _lightboxIndexLabel.Text = _lightboxPaths.Count > 1
                ? $"{index + 1} / {_lightboxPaths.Count}"
                : string.Empty;
        }
        // #3927: show the current image's file name (mirrors Obsidian 1.13.4).
        // Tooltip carries the full path so long file names stay inspectable.
        if (_lightboxFileNameLabel is not null)
        {
            var path = _lightboxPaths[index];
            var fileName = System.IO.Path.GetFileName(path);
            _lightboxFileNameLabel.Text = string.IsNullOrEmpty(fileName) ? path : fileName;
            ToolTipService.SetToolTip(_lightboxFileNameLabel, path);
        }
    }

    private void UpdateLightboxNavButtons()
    {
        var multi = _lightboxPaths.Count > 1;
        if (_lightboxPrevBtn is not null)
        {
            _lightboxPrevBtn.Visibility = multi ? Visibility.Visible : Visibility.Collapsed;
        }
        if (_lightboxNextBtn is not null)
        {
            _lightboxNextBtn.Visibility = multi ? Visibility.Visible : Visibility.Collapsed;
        }
    }

    // ── Zoom ────────────────────────────────────────────────────────
    private void Lightbox_SetZoom(double target)
    {
        if (_lightboxScale is null)
        {
            return;
        }
        var clamped = Math.Clamp(target, LightboxMinZoom, LightboxMaxZoom);
        _lightboxScale.ScaleX = clamped;
        _lightboxScale.ScaleY = clamped;
        if (_lightboxZoomLabel is not null)
        {
            _lightboxZoomLabel.Text = $"{(int)Math.Round(clamped * 100)}%";
        }
        // Reset pan when returning to 1x.
        if (clamped <= LightboxMinZoom && _lightboxTranslate is not null)
        {
            _lightboxTranslate.X = 0;
            _lightboxTranslate.Y = 0;
        }
    }

    private void Lightbox_ZoomIn(object sender, RoutedEventArgs e) =>
        Lightbox_SetZoom((_lightboxScale?.ScaleX ?? 1) + LightboxZoomStep);

    private void Lightbox_ZoomOut(object sender, RoutedEventArgs e) =>
        Lightbox_SetZoom((_lightboxScale?.ScaleX ?? 1) - LightboxZoomStep);

    private void Lightbox_ResetZoom() => Lightbox_SetZoom(1);

    private void Lightbox_ResetZoom(object sender, RoutedEventArgs e) => Lightbox_ResetZoom();

    // ── Navigation ──────────────────────────────────────────────────
    private void Lightbox_Navigate(int delta)
    {
        if (_lightboxPaths.Count <= 1)
        {
            return;
        }
        var next = (_lightboxIndex + delta + _lightboxPaths.Count) % _lightboxPaths.Count;
        if (next == _lightboxIndex)
        {
            return;
        }
        _lightboxIndex = next;
        _ = LoadLightboxImageAsync(_lightboxIndex);
    }

    // ── Pan (pointer drag when zoomed in) ───────────────────────────
    private void Lightbox_OnPointerPressed(object sender, PointerRoutedEventArgs e)
    {
        if (_lightboxScale is null || _lightboxScale.ScaleX <= LightboxMinZoom)
        {
            return;
        }
        _lightboxPanning = true;
        _lightboxPanLast = e.GetCurrentPoint((UIElement?)sender).Position;
        ((UIElement?)sender)?.CapturePointer(e.Pointer);
        e.Handled = true;
    }

    private void Lightbox_OnPointerMoved(object sender, PointerRoutedEventArgs e)
    {
        if (!_lightboxPanning || _lightboxTranslate is null)
        {
            return;
        }
        var pos = e.GetCurrentPoint((UIElement?)sender).Position;
        _lightboxTranslate.X += pos.X - _lightboxPanLast.X;
        _lightboxTranslate.Y += pos.Y - _lightboxPanLast.Y;
        _lightboxPanLast = pos;
        e.Handled = true;
    }

    private void Lightbox_OnPointerReleased(object sender, PointerRoutedEventArgs e)
    {
        if (_lightboxPanning)
        {
            _lightboxPanning = false;
            ((UIElement?)sender)?.ReleasePointerCapture(e.Pointer);
            e.Handled = true;
        }
    }

    // ── Swipe-to-dismiss (#3751) ────────────────────────────────────
    private void Lightbox_OnManipulationDelta(object sender, ManipulationDeltaRoutedEventArgs e)
    {
        // Only enable swipe-to-dismiss when not zoomed in (at 1x, pan is
        // not needed — the image fits the viewport).  When zoomed, let
        // pointer-drag pan handle it.
        if (_lightboxScale is { ScaleX: > LightboxMinZoom })
        {
            return;
        }

        _lightboxSwipeY += e.Delta.Translation.Y;
        _lightboxSwipeActive = true;

        // Visually follow the finger by translating the image container.
        if (_lightboxTranslate is not null)
        {
            _lightboxTranslate.Y = _lightboxSwipeY;
        }

        // Fade the background as the swipe progresses (0→1 opacity drops
        // toward 0.3 at ~200 px).
        var opacity = Math.Max(0.3, 1.0 - Math.Abs(_lightboxSwipeY) / 300.0);
        if (sender is Grid grid)
        {
            grid.Opacity = opacity;
        }
    }

    private void Lightbox_OnManipulationCompleted(object sender, ManipulationCompletedRoutedEventArgs e)
    {
        if (!_lightboxSwipeActive)
            return;

        _lightboxSwipeActive = false;

        // If the vertical swipe exceeded the dismiss threshold (120 px),
        // close the lightbox. Otherwise, animate back to original position.
        if (Math.Abs(_lightboxSwipeY) > 120)
        {
            _lightboxDialog?.Hide();
        }
        else
        {
            // Animate back — simple linear animation with 200ms duration.
            var translate = _lightboxTranslate;
            var grid = sender as Grid;
            if (translate is null || grid is null) return;

            var from = _lightboxSwipeY;
            var to = 0.0;
            var startTime = DateTime.UtcNow;
            var duration = TimeSpan.FromMilliseconds(200);

            var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(16) };
            timer.Tick += (_, _) =>
            {
                var elapsed = (DateTime.UtcNow - startTime).TotalMilliseconds;
                var t = Math.Clamp(elapsed / duration.TotalMilliseconds, 0, 1);
                // Ease-out cubic
                var eased = 1 - Math.Pow(1 - t, 3);
                translate.Y = from + (to - from) * eased;
                grid.Opacity = Math.Max(0.3, 1.0 - Math.Abs(translate.Y) / 300.0);

                if (t >= 1)
                {
                    translate.Y = 0;
                    grid.Opacity = 1;
                    timer.Stop();
                }
            };
            timer.Start();
        }

        _lightboxSwipeY = 0;
    }

    private void Lightbox_OnPointerWheel(object sender, PointerRoutedEventArgs e)
    {
        // Ctrl + wheel to zoom (matches the issue spec).
        if (!e.KeyModifiers.HasFlag(VirtualKeyModifiers.Control))
        {
            return;
        }
        var delta = e.GetCurrentPoint((UIElement?)sender).Properties.MouseWheelDelta;
        var factor = delta > 0 ? LightboxZoomStep : -LightboxZoomStep;
        Lightbox_SetZoom((_lightboxScale?.ScaleX ?? 1) + factor);
        e.Handled = true;
    }

    // ── Keyboard ────────────────────────────────────────────────────
    private void Lightbox_OnKeyDown(object sender, KeyRoutedEventArgs e)
    {
        switch (e.Key)
        {
            case VirtualKey.Left:
                Lightbox_Navigate(-1);
                e.Handled = true;
                break;
            case VirtualKey.Right:
                Lightbox_Navigate(1);
                e.Handled = true;
                break;
            case VirtualKey.Add:
            case (VirtualKey)187: // '+' / '=' key on many layouts
                Lightbox_ZoomIn(sender, e);
                e.Handled = true;
                break;
            case VirtualKey.Subtract:
            case (VirtualKey)189: // '-' key
                Lightbox_ZoomOut(sender, e);
                e.Handled = true;
                break;
            case (VirtualKey)48: // '0' → reset
                Lightbox_ResetZoom(sender, e);
                e.Handled = true;
                break;
            case VirtualKey.Escape:
                Lightbox_Close(sender, e);
                e.Handled = true;
                break;
        }
    }

    private void Lightbox_Close(object sender, RoutedEventArgs e)
    {
        _lightboxDialog?.Hide();
    }

    private async void Lightbox_Remove(object sender, RoutedEventArgs e)
    {
        var idx = _lightboxIndex;
        var removedPath = idx >= 0 && idx < _lightboxPaths.Count ? _lightboxPaths[idx] : null;

        // Close the lightbox first so the dialog isn't re-entered while the
        // caller mutates the underlying attachment list.
        _lightboxDialog?.Hide();

        if (_removeAttachmentByPathAction is not null)
        {
            try
            {
                await _removeAttachmentByPathAction(removedPath);
            }
            catch
            {
                // Best-effort; the caller owns the attachment list.
            }
        }
    }

    /// <summary>
    /// Hook set by <see cref="MainWindow.Attachments"/> so the lightbox can
    /// request removal of the currently-viewed image without taking a direct
    /// dependency on the attachment data model.
    /// </summary>
    private Func<string?, Task>? _removeAttachmentByPathAction;
}
