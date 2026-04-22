using Forge.Gui.App.Dialogs;
using Forge.Gui.App.Pages;
using Forge.Gui.App.Services;
using Forge.Gui.Core.Services;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Storage.Pickers;
using WinRT.Interop;

namespace Forge.Gui.App;

public sealed partial class MainWindow : Window
{
    private readonly IUiModeService _mode;
    private readonly IRepoRegistry _repos;
    private readonly OpenRepoService _open;
    private readonly ISettingsStore _settings;

    public MainWindow()
    {
        InitializeComponent();
        _mode = App.Services.GetRequiredService<IUiModeService>();
        _repos = App.Services.GetRequiredService<IRepoRegistry>();
        _open = App.Services.GetRequiredService<OpenRepoService>();
        _settings = App.Services.GetRequiredService<ISettingsStore>();

        // All event handlers that touch XAML must marshal to the UI
        // dispatcher — the events fire from Task.Run continuations
        // inside OpenRepoService, and touching a UIElement from a
        // background thread throws an empty-message COMException.
        _mode.ModeChanged += (_, _) =>
            DispatcherQueue.TryEnqueue(UpdateAdvancedVisibility);
        _repos.ActiveChanged += (_, r) =>
            DispatcherQueue.TryEnqueue(() =>
            {
                RepoSwitcherLabel.Text = r is null ? "No repo" : r.DisplayName;
            });

        TrySetMica();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        Title = "Forge";

        if (AppWindow?.TitleBar is not null)
        {
            AppWindow.TitleBar.ExtendsContentIntoTitleBar = true;
            RightPaddingColumn.Width = new GridLength(AppWindow.TitleBar.RightInset);
            LeftPaddingColumn.Width = new GridLength(AppWindow.TitleBar.LeftInset);
        }

        ContentFrame.Navigate(typeof(HomePage));
        UpdateAdvancedVisibility();
        UpdateRepoDependentNav(isOpen: _open.IsWorkspaceOpen);
        _open.WorkspaceOpened += (_, info) =>
        {
            OpenRepoService.DiagLog($"WorkspaceOpened event, enqueueing UI updates");
            DispatcherQueue.TryEnqueue(() =>
            {
                try
                {
                    OpenRepoService.DiagLog("UI thread: updating nav + navigating to Changes");
                    UpdateRepoDependentNav(isOpen: true);
                    ContentFrame.Navigate(typeof(ChangesPage));
                    foreach (var item in Nav.MenuItems)
                        if (item is NavigationViewItem nvi && (nvi.Tag as string) == "changes")
                        {
                            Nav.SelectedItem = nvi;
                            break;
                        }
                    OpenRepoService.DiagLog("UI thread: nav done");
                }
                catch (Exception ex)
                {
                    OpenRepoService.DiagLog($"UI thread: nav FAILED [{ex.GetType().Name}] {ex.Message}");
                }
            });
        };

        // Auto-reopen last repo is off for now — startup-time navigate
        // to ChangesPage before the XamlRoot/content tree is ready can
        // race. User can pick from the Home page (Recent list).
    }

    private void UpdateRepoDependentNav(bool isOpen)
    {
        // Gray out (disable) per-repo nav items when there's no
        // workspace. Home + Settings remain reachable.
        foreach (var item in Nav.MenuItems)
        {
            if (item is not NavigationViewItem nvi || nvi.Tag is not string tag) continue;
            if (tag is "changes" or "history" or "locks" or "branches"
                    or "stash" or "tags" or "stats")
            {
                nvi.IsEnabled = isOpen;
                nvi.Opacity = isOpen ? 1.0 : 0.45;
            }
        }
    }

    private async Task TryReopenLastAsync()
    {
        var last = _settings.Current.LastActiveRepoPath;
        if (string.IsNullOrEmpty(last)) return;
        if (!Directory.Exists(last)) return;
        try { await _open.OpenAsync(last); }
        catch
        {
            // Swallow — if the last repo is no longer a valid workspace,
            // fall back to the mock client without interrupting startup.
        }
    }

    private void TrySetMica()
    {
        if (MicaController.IsSupported())
        {
            SystemBackdrop = new MicaBackdrop { Kind = MicaKind.Base };
        }
    }

    private void UpdateAdvancedVisibility()
    {
        var adv = _mode.Current == UiMode.Advanced ? Visibility.Visible : Visibility.Collapsed;
        AdvancedSep.Visibility = adv;
        StashItem.Visibility = adv;
        TagsItem.Visibility = adv;
        StatsItem.Visibility = adv;
    }

    private void OnNavSelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        if (args.SelectedItem is not NavigationViewItem item || item.Tag is not string tag) return;

        Type? page = tag switch
        {
            "home"     => typeof(HomePage),
            "changes"  => typeof(ChangesPage),
            "history"  => typeof(HistoryPage),
            "locks"    => typeof(LocksPage),
            "branches" => typeof(BranchesPage),
            "stash"    => typeof(StubPage),
            "tags"     => typeof(StubPage),
            "stats"    => typeof(StubPage),
            _          => null,
        };
        if (page is not null) ContentFrame.Navigate(page, tag);
    }

    private void OnRefreshClicked(object sender, RoutedEventArgs e)
    {
        if (ContentFrame.Content is IRefreshable r) _ = r.RefreshAsync();
    }

    private void OnSettingsClicked(object sender, RoutedEventArgs e)
    {
        ContentFrame.Navigate(typeof(SettingsPage));
    }

    private void OnRepoMenuOpening(object? sender, object e)
    {
        // Rebuild the "recent repos" section of the menu each time
        // it's opened so it reflects the latest SettingsStore state.
        var items = RepoMenu.Items;
        for (int i = items.Count - 1; i >= 0; i--)
        {
            if (items[i] is MenuFlyoutItem mi && mi.Tag is string tagStr && tagStr.StartsWith("recent:"))
                items.RemoveAt(i);
        }

        var recent = _settings.Current.RecentRepos;
        if (recent.Count == 0)
        {
            RecentSeparator.Visibility = Visibility.Collapsed;
            return;
        }
        RecentSeparator.Visibility = Visibility.Visible;
        int insertAt = 0;
        foreach (var r in recent.Take(8))
        {
            var item = new MenuFlyoutItem
            {
                Text = r.DisplayName,
                Icon = new SymbolIcon(Symbol.Folder),
                Tag = "recent:" + r.Path,
            };
            item.Click += async (_, _) =>
            {
                try { await _open.OpenAsync(r.Path); }
                catch (Exception ex) { await ShowErrorAsync("Could not open repo", ex.Message); }
            };
            items.Insert(insertAt++, item);
        }
    }

    private async void OnOpenFolderClicked(object sender, RoutedEventArgs e)
    {
        var picker = new FolderPicker();
        InitializeWithWindow.Initialize(picker, WindowNative.GetWindowHandle(this));
        picker.SuggestedStartLocation = PickerLocationId.Desktop;
        picker.FileTypeFilter.Add("*");

        var folder = await picker.PickSingleFolderAsync();
        if (folder is null) return;
        try { await _open.OpenAsync(folder.Path); }
        catch (Exception ex)
        {
            await ShowErrorAsync(
                "Could not open workspace",
                $"{ex.Message}\n\nPath: {folder.Path}");
        }
    }

    private async void OnCloneClicked(object sender, RoutedEventArgs e)
    {
        var dlg = new CloneDialog { XamlRoot = Content.XamlRoot };
        await dlg.ShowAsync();
    }

    private Task ShowErrorAsync(string title, string detail)
    {
        var dlg = new ContentDialog
        {
            Title = title,
            Content = detail,
            CloseButtonText = "OK",
            XamlRoot = Content.XamlRoot,
        };
        return dlg.ShowAsync().AsTask();
    }
}

public interface IRefreshable
{
    Task RefreshAsync();
}
