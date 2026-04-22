using System.Runtime.CompilerServices;
using System.Threading.Channels;
using Forge.Gui.Core.Models;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Mock;

public sealed class MockForgeClient : IForgeClient
{
    private readonly List<FileChange> _changes = new(Fixtures.Changes);
    private readonly List<LogEntry> _log = new(Fixtures.Log);
    private readonly List<Branch> _branches = new(Fixtures.Branches);
    private readonly List<LockInfo> _locks = new(Fixtures.Locks);
    private readonly Channel<LockEvent> _lockEvents = Channel.CreateUnbounded<LockEvent>();

    public Task<WorkspaceInfo> OpenAsync(string path, CancellationToken ct = default)
        => Task.FromResult(Fixtures.Workspace with { Path = path });

    public Task<StatusResult> GetStatusAsync(CancellationToken ct = default)
    {
        var conflicts = _changes.Any(c => c.Kind == FileChangeKind.Conflicted);
        return Task.FromResult(new StatusResult(_changes.ToList(), Ahead: 2, Behind: 0, HasConflicts: conflicts));
    }

    public Task<IReadOnlyList<LogEntry>> GetLogAsync(int limit, string? branch, CancellationToken ct = default)
        => Task.FromResult<IReadOnlyList<LogEntry>>(_log.Take(limit).ToList());

    public Task<IReadOnlyList<Branch>> GetBranchesAsync(bool includeRemote, CancellationToken ct = default)
        => Task.FromResult<IReadOnlyList<Branch>>(
            includeRemote ? _branches.ToList() : _branches.Where(b => !b.IsRemote).ToList());

    public Task<IReadOnlyList<LockInfo>> GetLocksAsync(CancellationToken ct = default)
        => Task.FromResult<IReadOnlyList<LockInfo>>(_locks.ToList());

    public Task<DiffResult> GetDiffAsync(string path, DiffKind kind, CancellationToken ct = default)
    {
        if (path.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase) ||
            path.EndsWith(".umap",   StringComparison.OrdinalIgnoreCase))
        {
            return Task.FromResult(new DiffResult(
                Path: path,
                IsBinary: true,
                Hunks: Array.Empty<DiffHunk>(),
                AssetDeltas:
                [
                    new("Properties.Scale",        "(1, 1, 1)", "(1.2, 1.2, 1.2)"),
                    new("Properties.Material",     "M_Hero_v1", "M_Hero_v2"),
                    new("Properties.MaxHealth",    "100",       "125"),
                ]));
        }

        return Task.FromResult(new DiffResult(
            Path: path,
            IsBinary: false,
            Hunks:
            [
                new(10, 3, 10, 4,
                    "@@ -10,3 +10,4 @@\n void AHeroChar::Dash()\n {\n-    Velocity = FVector(500, 0, 0);\n+    Velocity = FVector(800, 0, 0);\n+    PlayDashEffect();\n }\n"),
            ],
            AssetDeltas: Array.Empty<AssetPropertyDelta>()));
    }

    public Task<AssetMetadata> GetAssetInfoAsync(string path, CancellationToken ct = default)
        => Task.FromResult(new AssetMetadata(
            Path: path,
            AssetClass: "SkeletalMesh",
            EngineVersion: "5.7.0",
            FileSize: 12_478_392,
            Dependencies: ["/Engine/BasicShapes/Cube", "/Game/Characters/Hero/M_Hero_v2"]));

    public Task StageAsync(IEnumerable<string> paths, CancellationToken ct = default)
    {
        foreach (var p in paths)
        {
            var i = _changes.FindIndex(c => c.Path == p);
            if (i >= 0)
            {
                var old = _changes[i];
                var staged = old.Kind switch
                {
                    FileChangeKind.Modified  => FileChangeKind.StagedModified,
                    FileChangeKind.Deleted   => FileChangeKind.StagedDeleted,
                    FileChangeKind.Untracked => FileChangeKind.StagedNew,
                    _                        => old.Kind,
                };
                _changes[i] = old with { Kind = staged };
            }
        }
        return Task.CompletedTask;
    }

    public Task UnstageAsync(IEnumerable<string> paths, CancellationToken ct = default)
    {
        foreach (var p in paths)
        {
            var i = _changes.FindIndex(c => c.Path == p);
            if (i >= 0)
            {
                var old = _changes[i];
                var unstaged = old.Kind switch
                {
                    FileChangeKind.StagedModified => FileChangeKind.Modified,
                    FileChangeKind.StagedDeleted  => FileChangeKind.Deleted,
                    FileChangeKind.StagedNew      => FileChangeKind.Untracked,
                    _                             => old.Kind,
                };
                _changes[i] = old with { Kind = unstaged };
            }
        }
        return Task.CompletedTask;
    }

    public Task<CommitResult> CommitAsync(string message, CancellationToken ct = default)
    {
        var staged = _changes.Where(c =>
            c.Kind is FileChangeKind.StagedNew or FileChangeKind.StagedModified or FileChangeKind.StagedDeleted).ToList();
        if (staged.Count == 0) return Task.FromException<CommitResult>(new InvalidOperationException("Nothing staged."));

        var hash = Guid.NewGuid().ToString("N")[..8];
        var parent = _log.Count > 0 ? _log[0].Hash : string.Empty;
        var entry = new LogEntry(
            hash,
            string.IsNullOrEmpty(parent) ? Array.Empty<string>() : new[] { parent },
            "Krishna Teja", "krishna@kriaa.in", DateTimeOffset.Now, message);
        _log.Insert(0, entry);

        foreach (var c in staged) _changes.Remove(c);
        return Task.FromResult(new CommitResult(hash, message));
    }

    public async Task PushAsync(IProgress<PushProgress>? progress = null, CancellationToken ct = default)
    {
        for (int i = 0; i <= 10; i++)
        {
            progress?.Report(new PushProgress("uploading", i, 10));
            await Task.Delay(80, ct).ConfigureAwait(false);
        }
    }

    public async Task PullAsync(IProgress<PullProgress>? progress = null, CancellationToken ct = default)
    {
        for (int i = 0; i <= 10; i++)
        {
            progress?.Report(new PullProgress("fetching", i, 10));
            await Task.Delay(80, ct).ConfigureAwait(false);
        }
    }

    public Task LockAsync(string path, string? reason, CancellationToken ct = default)
    {
        if (_locks.Any(l => l.Path == path)) return Task.CompletedTask;
        var info = new LockInfo(path, "krishna", DateTimeOffset.Now, reason);
        _locks.Add(info);
        _lockEvents.Writer.TryWrite(new LockEvent(LockEventKind.Acquired, info));
        return Task.CompletedTask;
    }

    public Task UnlockAsync(string path, bool force, CancellationToken ct = default)
    {
        var idx = _locks.FindIndex(l => l.Path == path);
        if (idx >= 0)
        {
            var info = _locks[idx];
            _locks.RemoveAt(idx);
            _lockEvents.Writer.TryWrite(new LockEvent(LockEventKind.Released, info));
        }
        return Task.CompletedTask;
    }

    public Task SwitchBranchAsync(string name, bool create, CancellationToken ct = default)
    {
        for (int i = 0; i < _branches.Count; i++)
        {
            var b = _branches[i];
            _branches[i] = b with { IsCurrent = !b.IsRemote && b.Name == name };
        }
        if (create && _branches.All(b => b.Name != name))
            _branches.Insert(0, new Branch(name, true, false, null, 0, 0));
        return Task.CompletedTask;
    }

    public Task CreateBranchAsync(string name, CancellationToken ct = default)
    {
        if (_branches.All(b => b.Name != name))
            _branches.Insert(0, new Branch(name, false, false, null, 0, 0));
        return Task.CompletedTask;
    }

    public Task DeleteBranchAsync(string name, bool force, CancellationToken ct = default)
    {
        _branches.RemoveAll(b => b.Name == name && !b.IsRemote);
        return Task.CompletedTask;
    }

    public Task<CloneResult> CloneAsync(string url, string dest, IProgress<CloneProgress>? progress = null, CancellationToken ct = default)
        => Task.FromResult(new CloneResult(dest, new Uri(url).Segments.Last().TrimEnd('/')));

    public Task LoginAsync(string serverUrl, string user, string token, CancellationToken ct = default)
        => Task.CompletedTask;

    public async IAsyncEnumerable<LockEvent> SubscribeLockEventsAsync([EnumeratorCancellation] CancellationToken ct = default)
    {
        while (await _lockEvents.Reader.WaitToReadAsync(ct).ConfigureAwait(false))
            while (_lockEvents.Reader.TryRead(out var evt))
                yield return evt;
    }
}
