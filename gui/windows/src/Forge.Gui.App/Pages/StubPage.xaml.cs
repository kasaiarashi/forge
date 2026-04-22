using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;

namespace Forge.Gui.App.Pages;

public sealed partial class StubPage : Page
{
    public StubPage() { InitializeComponent(); }

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        if (e.Parameter is string tag)
            Label.Text = $"{char.ToUpper(tag[0])}{tag[1..]} — coming in a later phase";
    }
}
