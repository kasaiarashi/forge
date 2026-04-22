using Forge.Gui.Core.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml.Controls;

namespace Forge.Gui.App.Pages;

public sealed partial class SettingsPage : Page
{
    public SettingsViewModel Vm { get; }

    public SettingsPage()
    {
        Vm = App.Services.GetRequiredService<SettingsViewModel>();
        InitializeComponent();
    }
}
