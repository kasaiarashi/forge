using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Forge.Gui.Core.Models;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Core.ViewModels;

public sealed partial class LocksViewModel : ObservableObject
{
    private readonly IForgeClient _client;

    public ObservableCollection<LockInfo> Locks { get; } = new();

    [ObservableProperty]
    private bool _onlyMine;

    [ObservableProperty]
    private bool _isBusy;

    public LocksViewModel(IForgeClient client) { _client = client; }

    [RelayCommand]
    public async Task RefreshAsync(CancellationToken ct = default)
    {
        IsBusy = true;
        try
        {
            Locks.Clear();
            var locks = await _client.GetLocksAsync(ct);
            foreach (var l in locks) Locks.Add(l);
        }
        finally { IsBusy = false; }
    }

    [RelayCommand]
    public async Task ReleaseAsync(LockInfo info)
    {
        await _client.UnlockAsync(info.Path, force: false);
        await RefreshAsync();
    }
}
