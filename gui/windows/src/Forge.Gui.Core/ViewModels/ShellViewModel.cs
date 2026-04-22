using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Core.ViewModels;

public sealed partial class ShellViewModel : ObservableObject
{
    private readonly IRepoRegistry _repos;
    private readonly IUiModeService _mode;

    [ObservableProperty]
    private RepoEntry? _activeRepo;

    [ObservableProperty]
    private UiMode _currentMode;

    public ObservableCollection<NavItem> NavItems { get; } = new();

    public ShellViewModel(IRepoRegistry repos, IUiModeService mode)
    {
        _repos = repos;
        _mode = mode;
        _activeRepo = repos.Active;
        _currentMode = mode.Current;

        repos.ActiveChanged += (_, r) => ActiveRepo = r;
        mode.ModeChanged += (_, m) => { CurrentMode = m; RebuildNav(); };

        RebuildNav();
    }

    private void RebuildNav()
    {
        NavItems.Clear();
        NavItems.Add(new NavItem("changes",   "Changes",   ""));
        NavItems.Add(new NavItem("history",   "History",   ""));
        NavItems.Add(new NavItem("locks",     "Locks",     ""));
        NavItems.Add(new NavItem("branches",  "Branches",  ""));
        if (CurrentMode == UiMode.Advanced)
        {
            NavItems.Add(new NavItem("stash",  "Stash",  ""));
            NavItems.Add(new NavItem("tags",   "Tags",   ""));
            NavItems.Add(new NavItem("stats",  "Stats",  ""));
        }
    }
}

public sealed record NavItem(string Tag, string Label, string IconGlyph);
