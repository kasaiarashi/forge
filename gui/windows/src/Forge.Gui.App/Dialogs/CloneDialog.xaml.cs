using Forge.Gui.Core.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml.Controls;

namespace Forge.Gui.App.Dialogs;

public sealed partial class CloneDialog : ContentDialog
{
    public CloneViewModel Vm { get; }

    public CloneDialog()
    {
        Vm = App.Services.GetRequiredService<CloneViewModel>();
        InitializeComponent();
    }

    private async void OnPrimaryClicked(ContentDialog sender, ContentDialogButtonClickEventArgs args)
    {
        var deferral = args.GetDeferral();
        try
        {
            await Vm.StartCommand.ExecuteAsync(null);
            if (Vm.ErrorMessage is not null) args.Cancel = true;
        }
        finally { deferral.Complete(); }
    }
}
