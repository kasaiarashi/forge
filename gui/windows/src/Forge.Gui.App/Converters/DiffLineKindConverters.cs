using Forge.Gui.Core.ViewModels;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Media;
using Windows.UI;

namespace Forge.Gui.App.Converters;

public sealed class DiffLineKindToBackgroundConverter : IValueConverter
{
    private static readonly SolidColorBrush Added    = new(Color.FromArgb(48, 108, 203, 95));
    private static readonly SolidColorBrush Removed  = new(Color.FromArgb(48, 248, 81, 73));
    private static readonly SolidColorBrush Hunk     = new(Color.FromArgb(24, 210, 168, 255));
    private static readonly SolidColorBrush Context  = new(Color.FromArgb(0, 0, 0, 0));

    public object Convert(object value, Type targetType, object parameter, string language)
    {
        return value switch
        {
            DiffLineKind.Added   => Added,
            DiffLineKind.Removed => Removed,
            DiffLineKind.Hunk    => Hunk,
            _                    => Context,
        };
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
        => throw new NotImplementedException();
}

public sealed class DiffLineNumberFormatter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
        => value is int n ? n.ToString() : " ";

    public object ConvertBack(object value, Type targetType, object parameter, string language)
        => throw new NotImplementedException();
}
