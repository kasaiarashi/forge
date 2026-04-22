using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Forge.Gui.Core.Models;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Core.ViewModels;

public sealed partial class DiffViewModel : ObservableObject
{
    private readonly IForgeClient _client;

    [ObservableProperty] private string? _path;
    [ObservableProperty] private bool _isBinary;
    [ObservableProperty] private bool _isBusy;
    [ObservableProperty] private string? _headerLabel;
    [ObservableProperty] private AssetMetadata? _assetMetadata;

    public ObservableCollection<DiffHunkLine> Lines { get; } = new();
    public ObservableCollection<AssetPropertyDelta> AssetDeltas { get; } = new();

    public DiffViewModel(IForgeClient client) { _client = client; }

    [RelayCommand]
    public async Task LoadAsync((string path, DiffKind kind) args, CancellationToken ct = default)
    {
        Path = args.path;
        IsBusy = true;
        Lines.Clear();
        AssetDeltas.Clear();
        HeaderLabel = args.path;
        try
        {
            var diff = await _client.GetDiffAsync(args.path, args.kind, ct).ConfigureAwait(false);
            IsBinary = diff.IsBinary;
            if (diff.IsBinary)
            {
                foreach (var d in diff.AssetDeltas) AssetDeltas.Add(d);
                try
                {
                    AssetMetadata = await _client.GetAssetInfoAsync(args.path, ct).ConfigureAwait(false);
                }
                catch { AssetMetadata = null; }
            }
            else
            {
                foreach (var h in diff.Hunks)
                foreach (var line in ParseHunk(h))
                    Lines.Add(line);
            }
        }
        finally { IsBusy = false; }
    }

    private static IEnumerable<DiffHunkLine> ParseHunk(DiffHunk hunk)
    {
        int oldLine = hunk.OldStart;
        int newLine = hunk.NewStart;
        foreach (var rawLine in hunk.Body.Split('\n'))
        {
            if (rawLine.StartsWith("@@", StringComparison.Ordinal))
            {
                yield return new DiffHunkLine(DiffLineKind.Hunk, null, null, rawLine);
                continue;
            }
            if (rawLine.Length == 0) continue;

            var tag = rawLine[0];
            var text = rawLine[1..];
            switch (tag)
            {
                case '+': yield return new DiffHunkLine(DiffLineKind.Added,   null,    newLine++, text); break;
                case '-': yield return new DiffHunkLine(DiffLineKind.Removed, oldLine++, null,   text); break;
                case ' ': yield return new DiffHunkLine(DiffLineKind.Context, oldLine++, newLine++, text); break;
                default:  yield return new DiffHunkLine(DiffLineKind.Context, null,    null,   rawLine); break;
            }
        }
    }
}

public enum DiffLineKind { Context, Added, Removed, Hunk }

public sealed record DiffHunkLine(DiffLineKind Kind, int? OldLineNo, int? NewLineNo, string Text);
