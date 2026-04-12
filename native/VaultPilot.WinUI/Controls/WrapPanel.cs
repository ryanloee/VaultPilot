using System;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;

namespace VaultPilot.WinUI.Controls;

public sealed class WrapPanel : Panel
{
    public static readonly DependencyProperty OrientationProperty =
        DependencyProperty.Register(
            nameof(Orientation),
            typeof(Orientation),
            typeof(WrapPanel),
            new PropertyMetadata(Orientation.Horizontal, OnLayoutPropertyChanged));

    public static readonly DependencyProperty ItemWidthProperty =
        DependencyProperty.Register(
            nameof(ItemWidth),
            typeof(double),
            typeof(WrapPanel),
            new PropertyMetadata(0d, OnLayoutPropertyChanged));

    public static readonly DependencyProperty ItemHeightProperty =
        DependencyProperty.Register(
            nameof(ItemHeight),
            typeof(double),
            typeof(WrapPanel),
            new PropertyMetadata(0d, OnLayoutPropertyChanged));

    public Orientation Orientation
    {
        get => (Orientation)GetValue(OrientationProperty);
        set => SetValue(OrientationProperty, value);
    }

    public double ItemWidth
    {
        get => (double)GetValue(ItemWidthProperty);
        set => SetValue(ItemWidthProperty, value);
    }

    public double ItemHeight
    {
        get => (double)GetValue(ItemHeightProperty);
        set => SetValue(ItemHeightProperty, value);
    }

    protected override Size MeasureOverride(Size availableSize)
    {
        var lineSize = new Size();
        var totalSize = new Size();
        var itemWidth = ItemWidth;
        var itemHeight = ItemHeight;
        var isHorizontal = Orientation == Orientation.Horizontal;
        var maxPrimary = isHorizontal ? availableSize.Width : availableSize.Height;

        foreach (var child in Children)
        {
            if (child is null)
            {
                continue;
            }

            var constraint = new Size(
                itemWidth > 0 ? itemWidth : availableSize.Width,
                itemHeight > 0 ? itemHeight : availableSize.Height);
            child.Measure(constraint);

            var childSize = new Size(
                itemWidth > 0 ? itemWidth : child.DesiredSize.Width,
                itemHeight > 0 ? itemHeight : child.DesiredSize.Height);

            var primary = isHorizontal ? childSize.Width : childSize.Height;
            var secondary = isHorizontal ? childSize.Height : childSize.Width;

            if (lineSize.Primary(isHorizontal) + primary > maxPrimary && lineSize.Primary(isHorizontal) > 0)
            {
                totalSize = totalSize.AddLine(lineSize, isHorizontal);
                lineSize = new Size(primary, secondary).Swap(isHorizontal);
            }
            else
            {
                lineSize = lineSize.Grow(primary, secondary, isHorizontal);
            }
        }

        totalSize = totalSize.AddLine(lineSize, isHorizontal);
        return totalSize;
    }

    protected override Size ArrangeOverride(Size finalSize)
    {
        var isHorizontal = Orientation == Orientation.Horizontal;
        var itemWidth = ItemWidth;
        var itemHeight = ItemHeight;
        var maxPrimary = isHorizontal ? finalSize.Width : finalSize.Height;

        double linePrimary = 0;
        double lineSecondary = 0;
        double offsetPrimary = 0;
        double offsetSecondary = 0;

        foreach (var child in Children)
        {
            if (child is null)
            {
                continue;
            }

            var childSize = new Size(
                itemWidth > 0 ? itemWidth : child.DesiredSize.Width,
                itemHeight > 0 ? itemHeight : child.DesiredSize.Height);

            var primary = isHorizontal ? childSize.Width : childSize.Height;
            var secondary = isHorizontal ? childSize.Height : childSize.Width;

            if (offsetPrimary + primary > maxPrimary && linePrimary > 0)
            {
                offsetPrimary = 0;
                offsetSecondary += lineSecondary;
                lineSecondary = 0;
                linePrimary = 0;
            }

            var rect = isHorizontal
                ? new Rect(offsetPrimary, offsetSecondary, primary, secondary)
                : new Rect(offsetSecondary, offsetPrimary, secondary, primary);
            child.Arrange(rect);

            offsetPrimary += primary;
            linePrimary += primary;
            lineSecondary = Math.Max(lineSecondary, secondary);
        }

        return finalSize;
    }

    private static void OnLayoutPropertyChanged(DependencyObject d, DependencyPropertyChangedEventArgs e)
    {
        if (d is WrapPanel panel)
        {
            panel.InvalidateMeasure();
            panel.InvalidateArrange();
        }
    }
}

internal static class WrapPanelSizeExtensions
{
    public static double Primary(this Size size, bool horizontal) =>
        horizontal ? size.Width : size.Height;

    public static Size Swap(this Size size, bool horizontal) =>
        horizontal ? size : new Size(size.Height, size.Width);

    public static Size Grow(this Size size, double primary, double secondary, bool horizontal)
    {
        if (horizontal)
        {
            return new Size(size.Width + primary, Math.Max(size.Height, secondary));
        }

        return new Size(Math.Max(size.Width, secondary), size.Height + primary);
    }

    public static Size AddLine(this Size total, Size line, bool horizontal)
    {
        if (horizontal)
        {
            return new Size(Math.Max(total.Width, line.Width), total.Height + line.Height);
        }

        return new Size(total.Width + line.Width, Math.Max(total.Height, line.Height));
    }
}
