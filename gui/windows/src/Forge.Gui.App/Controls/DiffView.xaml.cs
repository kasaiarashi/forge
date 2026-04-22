using Forge.Gui.Core.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Forge.Gui.App.Controls;

public sealed partial class DiffView : UserControl
{
    public DiffViewModel Vm { get; }

    public DiffView()
    {
        Vm = App.Services.GetRequiredService<DiffViewModel>();
        InitializeComponent();
    }

    private void OnOpenInUeClicked(object sender, RoutedEventArgs e)
    {
        // Stub: P2+ will use SettingsStore.UnrealEditorPath + Process.Start.
        // For P0 the button exists so layout is stable.
    }
}
