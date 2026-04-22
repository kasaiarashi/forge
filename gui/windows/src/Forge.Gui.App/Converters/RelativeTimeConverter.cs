using Microsoft.UI.Xaml.Data;

namespace Forge.Gui.App.Converters;

/// <summary>
/// Formats a <see cref="DateTimeOffset"/> as a coarse relative string
/// ("2m ago", "3h ago", "5d ago") for recent commits, falling back to
/// an absolute yyyy-MM-dd date once the age exceeds ~14 days. Matches
/// the density most VCS UIs use in log views.
/// </summary>
public sealed class RelativeTimeConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        if (value is not DateTimeOffset ts) return string.Empty;
        var delta = DateTimeOffset.UtcNow - ts.ToUniversalTime();
        if (delta.TotalSeconds < 0) return ts.LocalDateTime.ToString("yyyy-MM-dd HH:mm");
        if (delta.TotalSeconds < 60) return "just now";
        if (delta.TotalMinutes < 60) return $"{(int)delta.TotalMinutes}m ago";
        if (delta.TotalHours < 24) return $"{(int)delta.TotalHours}h ago";
        if (delta.TotalDays < 14) return $"{(int)delta.TotalDays}d ago";
        return ts.LocalDateTime.ToString("yyyy-MM-dd");
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
        => throw new NotImplementedException();
}
