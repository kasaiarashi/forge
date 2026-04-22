using Forge.Gui.Core.Models;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Media;

namespace Forge.Gui.App.Converters;

public sealed class ChangeKindToBrushConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        if (value is not FileChangeKind k) return new SolidColorBrush();
        var key = k switch
        {
            FileChangeKind.StagedNew or FileChangeKind.Untracked => "FgAddedBrush",
            FileChangeKind.StagedModified or FileChangeKind.Modified => "FgModifiedBrush",
            FileChangeKind.StagedDeleted or FileChangeKind.Deleted => "FgDeletedBrush",
            FileChangeKind.Conflicted => "FgConflictBrush",
            _ => "FgUntrackedBrush",
        };
        return Application.Current.Resources[key]!;
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
        => throw new NotImplementedException();
}

public sealed class ChangeKindToGlyphConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        return value switch
        {
            FileChangeKind.StagedNew or FileChangeKind.Untracked            => "+",
            FileChangeKind.StagedModified or FileChangeKind.Modified        => "M",
            FileChangeKind.StagedDeleted or FileChangeKind.Deleted          => "−",
            FileChangeKind.Conflicted                                       => "!",
            _                                                               => "?",
        };
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
        => throw new NotImplementedException();
}

public sealed class BoolToVisibilityConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
        => value is true ? Visibility.Visible : Visibility.Collapsed;

    public object ConvertBack(object value, Type targetType, object parameter, string language)
        => value is Visibility.Visible;
}
