using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Core.ViewModels;

public sealed record HomeRecentRepo(string DisplayName, string Path, DateTimeOffset LastOpened, string AccentHex);

public sealed partial class HomeViewModel : ObservableObject
{
    private readonly ISettingsStore _settings;

    public ObservableCollection<HomeRecentRepo> Recents { get; } = new();

    [ObservableProperty] private bool _hasRecents;
    [ObservableProperty] private string _greeting = "Welcome to Forge";
    [ObservableProperty] private string _subtitle = "Version control built for Unreal Engine teams.";

    public HomeViewModel(ISettingsStore settings)
    {
        _settings = settings;
        // NOTE: No Settings.Changed subscription here. The Core VM has
        // no UI-thread context, and mutating a XAML-bound
        // ObservableCollection from a background-thread event handler
        // throws COMException. HomePage wires the subscription and
        // marshals Reload() onto the dispatcher.
        Reload();
    }

    public void Reload()
    {
        Recents.Clear();
        foreach (var r in _settings.Current.RecentRepos.OrderByDescending(r => r.LastOpened).Take(8))
        {
            Recents.Add(new HomeRecentRepo(r.DisplayName, r.Path, r.LastOpened, r.AccentHex));
        }
        HasRecents = Recents.Count > 0;

        var hour = DateTime.Now.Hour;
        Greeting = hour switch
        {
            < 5  => "Still up?",
            < 12 => "Good morning",
            < 17 => "Good afternoon",
            < 22 => "Good evening",
            _    => "Late shift",
        };
    }
}
