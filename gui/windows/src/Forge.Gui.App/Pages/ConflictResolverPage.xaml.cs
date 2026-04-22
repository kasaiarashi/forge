using Forge.Gui.Core.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml.Controls;

namespace Forge.Gui.App.Pages;

public sealed partial class ConflictResolverPage : Page
{
    public ConflictsViewModel Vm { get; }

    public ConflictResolverPage()
    {
        Vm = App.Services.GetRequiredService<ConflictsViewModel>();
        InitializeComponent();
    }
}
