using Forge.Gui.Core.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml.Controls;

namespace Forge.Gui.App.Pages;

public sealed partial class HistoryPage : Page, IRefreshable
{
    public HistoryViewModel Vm { get; }

    public HistoryPage()
    {
        Vm = App.Services.GetRequiredService<HistoryViewModel>();
        InitializeComponent();
        Loaded += async (_, _) => await Vm.LoadAsync();
    }

    public Task RefreshAsync() => Vm.LoadAsync();
}
