using Forge.Gui.Core.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml.Controls;

namespace Forge.Gui.App.Pages;

public sealed partial class BranchesPage : Page, IRefreshable
{
    public BranchesViewModel Vm { get; }

    public BranchesPage()
    {
        Vm = App.Services.GetRequiredService<BranchesViewModel>();
        InitializeComponent();
        Loaded += async (_, _) => await Vm.RefreshAsync();
    }

    public Task RefreshAsync() => Vm.RefreshAsync();
}
