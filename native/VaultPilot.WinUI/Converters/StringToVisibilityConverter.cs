using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Data;

namespace VaultPilot.WinUI.Converters;

/// <summary>
/// Converts a string value to <see cref="Visibility"/>:
/// non-empty → Visible, empty/null → Collapsed.
/// </summary>
public sealed class StringToVisibilityConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        return value is string s && !string.IsNullOrEmpty(s)
            ? Visibility.Visible
            : Visibility.Collapsed;
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
    {
        throw new NotSupportedException();
    }
}
