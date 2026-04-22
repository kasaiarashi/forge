using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Forge.Gui.Core.Models;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Core.ViewModels;

public sealed partial class ChangesViewModel : ObservableObject
{
    private readonly IForgeClient _client;

    public ObservableCollection<FileChange> Staged { get; } = new();
    public ObservableCollection<FileChange> Unstaged { get; } = new();
    public ObservableCollection<FileChange> Untracked { get; } = new();

    [ObservableProperty]
    private string _commitMessage = string.Empty;

    [ObservableProperty]
    private int _ahead;

    [ObservableProperty]
    private int _behind;

    [ObservableProperty]
    private bool _isBusy;

    [ObservableProperty]
    private int _total;

    public ChangesViewModel(IForgeClient client)
    {
        _client = client;
    }

    [RelayCommand]
    public async Task RefreshAsync(CancellationToken ct = default)
    {
        IsBusy = true;
        try
        {
            var status = await _client.GetStatusAsync(ct);
            Staged.Clear();
            Unstaged.Clear();
            Untracked.Clear();

            foreach (var c in status.Changes)
            {
                switch (c.Kind)
                {
                    case FileChangeKind.StagedNew:
                    case FileChangeKind.StagedModified:
                    case FileChangeKind.StagedDeleted:
                        Staged.Add(c); break;
                    case FileChangeKind.Modified:
                    case FileChangeKind.Deleted:
                    case FileChangeKind.Conflicted:
                        Unstaged.Add(c); break;
                    case FileChangeKind.Untracked:
                        Untracked.Add(c); break;
                }
            }
            Ahead = status.Ahead;
            Behind = status.Behind;
            Total = Staged.Count + Unstaged.Count + Untracked.Count;
        }
        finally { IsBusy = false; }
    }

    [RelayCommand]
    public async Task CommitAsync()
    {
        if (string.IsNullOrWhiteSpace(CommitMessage)) return;
        IsBusy = true;
        try
        {
            await _client.CommitAsync(CommitMessage);
            CommitMessage = string.Empty;
            await RefreshAsync();
        }
        finally { IsBusy = false; }
    }

    [RelayCommand]
    public async Task CommitAndPushAsync()
    {
        if (string.IsNullOrWhiteSpace(CommitMessage)) return;
        IsBusy = true;
        try
        {
            await _client.CommitAsync(CommitMessage);
            CommitMessage = string.Empty;
            await _client.PushAsync();
            await RefreshAsync();
        }
        finally { IsBusy = false; }
    }
}
