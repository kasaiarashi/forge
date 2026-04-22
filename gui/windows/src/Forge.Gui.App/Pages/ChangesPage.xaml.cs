using Forge.Gui.Core.Models;
using Forge.Gui.Core.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml.Controls;

namespace Forge.Gui.App.Pages;

public sealed partial class ChangesPage : Page, IRefreshable
{
    public ChangesViewModel Vm { get; }

    public ChangesPage()
    {
        Vm = App.Services.GetRequiredService<ChangesViewModel>();
        InitializeComponent();
        Loaded += async (_, _) => await Vm.RefreshAsync();
    }

    public Task RefreshAsync() => Vm.RefreshAsync();

    private async void OnChangeSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (e.AddedItems.Count == 0) return;
        if (e.AddedItems[0] is FileChange c)
        {
            var kind = c.Kind is FileChangeKind.StagedNew
                            or FileChangeKind.StagedModified
                            or FileChangeKind.StagedDeleted
                ? DiffKind.Staged
                : DiffKind.Working;
            await Diff.Vm.LoadAsync((c.Path, kind));
        }
    }
}
