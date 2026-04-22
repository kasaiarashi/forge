using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Forge.Gui.Core.Models;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Core.ViewModels;

public sealed partial class BranchesViewModel : ObservableObject
{
    private readonly IForgeClient _client;

    public ObservableCollection<Branch> Local { get; } = new();
    public ObservableCollection<Branch> Remote { get; } = new();

    [ObservableProperty]
    private bool _isBusy;

    public BranchesViewModel(IForgeClient client) { _client = client; }

    [RelayCommand]
    public async Task RefreshAsync(CancellationToken ct = default)
    {
        IsBusy = true;
        try
        {
            Local.Clear();
            Remote.Clear();
            var all = await _client.GetBranchesAsync(includeRemote: true, ct);
            foreach (var b in all)
            {
                if (b.IsRemote) Remote.Add(b);
                else Local.Add(b);
            }
        }
        finally { IsBusy = false; }
    }

    [RelayCommand]
    public async Task SwitchAsync(Branch b)
    {
        await _client.SwitchBranchAsync(b.Name, create: false);
        await RefreshAsync();
    }
}
