using Forge.Gui.Core.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml.Controls;

namespace Forge.Gui.App.Pages;

public sealed partial class LocksPage : Page, IRefreshable
{
    public LocksViewModel Vm { get; }

    public LocksPage()
    {
        Vm = App.Services.GetRequiredService<LocksViewModel>();
        InitializeComponent();
        Loaded += async (_, _) => await Vm.RefreshAsync();
    }

    public Task RefreshAsync() => Vm.RefreshAsync();
}
