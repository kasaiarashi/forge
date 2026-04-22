using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Forge.Gui.Core.Models;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Core.ViewModels;

public sealed partial class HistoryViewModel : ObservableObject
{
    private readonly IForgeClient _client;

    public ObservableCollection<LogEntry> Commits { get; } = new();

    [ObservableProperty]
    private LogEntry? _selected;

    [ObservableProperty]
    private bool _isBusy;

    [ObservableProperty]
    private int _total;

    public HistoryViewModel(IForgeClient client) { _client = client; }

    [RelayCommand]
    public async Task LoadAsync(CancellationToken ct = default)
    {
        IsBusy = true;
        try
        {
            Commits.Clear();
            // limit=0 means unlimited on the FFI side; the walk still
            // stops at the graph boundary. Users expect the full log,
            // not a silent 200-entry truncation.
            var log = await _client.GetLogAsync(0, null, ct);
            foreach (var c in log) Commits.Add(c);
            Total = Commits.Count;
        }
        finally { IsBusy = false; }
    }
}
