using Forge.Gui.App.Dialogs;
using Forge.Gui.App.Services;
using Forge.Gui.Core.Services;
using Forge.Gui.Core.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Storage.Pickers;
using WinRT.Interop;

namespace Forge.Gui.App.Pages;

public sealed partial class HomePage : Page
{
    public HomeViewModel Vm { get; }
    private readonly OpenRepoService _open;
    private readonly ISettingsStore _settings;
    private EventHandler? _settingsHandler;

    public HomePage()
    {
        Vm = App.Services.GetRequiredService<HomeViewModel>();
        _open = App.Services.GetRequiredService<OpenRepoService>();
        _settings = App.Services.GetRequiredService<ISettingsStore>();
        InitializeComponent();

        // Re-pull recents whenever settings change — but marshal the
        // Reload() onto the UI thread since ISettingsStore fires the
        // Changed event from whichever thread called UpdateAsync
        // (typically a Task.Run continuation), and ObservableCollection
        // mutations bound to XAML have to happen on the dispatcher.
        _settingsHandler = (_, _) =>
            DispatcherQueue.TryEnqueue(() => Vm.Reload());
        _settings.Changed += _settingsHandler;
        Unloaded += (_, _) =>
        {
            if (_settingsHandler is not null)
                _settings.Changed -= _settingsHandler;
        };
    }

    private async void OnOpenFolderClicked(object sender, RoutedEventArgs e)
    {
        var picker = new FolderPicker();
        var hwnd = WindowNative.GetWindowHandle(MainWindowInstance());
        InitializeWithWindow.Initialize(picker, hwnd);
        picker.SuggestedStartLocation = PickerLocationId.Desktop;
        picker.FileTypeFilter.Add("*");
        var folder = await picker.PickSingleFolderAsync();
        if (folder is null) return;
        try { await _open.OpenAsync(folder.Path); }
        catch (Exception ex)
        {
            await ShowErrorAsync("Could not open workspace", $"{ex.Message}\n\nPath: {folder.Path}");
        }
    }

    private async void OnCloneClicked(object sender, RoutedEventArgs e)
    {
        var dlg = new CloneDialog { XamlRoot = XamlRoot };
        await dlg.ShowAsync();
    }

    private void OnSignInClicked(object sender, RoutedEventArgs e)
    {
        Frame.Navigate(typeof(SettingsPage));
    }

    private async void OnRecentClicked(object sender, RoutedEventArgs e)
    {
        if (sender is not Button btn || btn.Tag is not string path) return;
        try { await _open.OpenAsync(path); }
        catch (Exception ex) { await ShowErrorAsync("Could not open workspace", $"{ex.Message}\n\nPath: {path}"); }
    }

    private Window MainWindowInstance() => (Application.Current as App)?.MainWindow ?? throw new InvalidOperationException("No main window");

    private Task ShowErrorAsync(string title, string detail)
    {
        var dlg = new ContentDialog
        {
            Title = title,
            Content = detail,
            CloseButtonText = "OK",
            XamlRoot = XamlRoot,
        };
        return dlg.ShowAsync().AsTask();
    }
}
