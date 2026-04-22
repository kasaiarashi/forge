using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Data;

namespace Forge.Gui.App.Converters;

/// <summary>
/// Visible when value is non-null/non-empty; collapsed otherwise.
/// Pass ConverterParameter="invert" to flip (visible when null/empty).
/// </summary>
public sealed class NullToCollapsedConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        var invert = parameter is string s && string.Equals(s, "invert", StringComparison.OrdinalIgnoreCase);
        var isPresent = value switch
        {
            null       => false,
            string str => !string.IsNullOrEmpty(str),
            _          => true,
        };
        if (invert) isPresent = !isPresent;
        return isPresent ? Visibility.Visible : Visibility.Collapsed;
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
        => throw new NotImplementedException();
}
