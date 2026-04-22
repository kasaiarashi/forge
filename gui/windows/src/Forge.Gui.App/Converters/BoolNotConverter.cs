using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Data;

namespace Forge.Gui.App.Converters;

public sealed class BoolNotConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        var b = value is bool x && x;
        if (targetType == typeof(Visibility))
            return b ? Visibility.Collapsed : Visibility.Visible;
        return !b;
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
        => Convert(value, targetType, parameter, language);
}
