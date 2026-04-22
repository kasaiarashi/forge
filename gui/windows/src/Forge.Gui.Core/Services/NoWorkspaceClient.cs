using System.Runtime.CompilerServices;
using Forge.Gui.Core.Models;

namespace Forge.Gui.Core.Services;

/// <summary>
/// Default <see cref="IForgeClient"/> sitting behind <see cref="ActiveForgeClient"/>
/// when the user hasn't opened a workspace yet. Every data-returning
/// op hands back an empty result (never throws) so ViewModels can
/// bind safely during the empty-state / HomePage view. Every mutating
/// op throws an <see cref="InvalidOperationException"/> so no
/// accidental write lands without a real session.
/// </summary>
public sealed class NoWorkspaceClient : IForgeClient
{
    private const string NoRepo = "No Forge workspace is open. Open a folder or clone one first.";

    public Task<WorkspaceInfo> OpenAsync(string path, CancellationToken ct = default)
        => Task.FromException<WorkspaceInfo>(new InvalidOperationException(
            "NoWorkspaceClient cannot open; the host must swap in a real client."));

    public Task<StatusResult> GetStatusAsync(CancellationToken ct = default)
        => Task.FromResult(new StatusResult(Array.Empty<FileChange>(), 0, 0, false));

    public Task<IReadOnlyList<LogEntry>> GetLogAsync(int limit, string? branch, CancellationToken ct = default)
        => Task.FromResult<IReadOnlyList<LogEntry>>(Array.Empty<LogEntry>());

    public Task<IReadOnlyList<Branch>> GetBranchesAsync(bool includeRemote, CancellationToken ct = default)
        => Task.FromResult<IReadOnlyList<Branch>>(Array.Empty<Branch>());

    public Task<IReadOnlyList<LockInfo>> GetLocksAsync(CancellationToken ct = default)
        => Task.FromResult<IReadOnlyList<LockInfo>>(Array.Empty<LockInfo>());

    public Task<DiffResult> GetDiffAsync(string path, DiffKind kind, CancellationToken ct = default)
        => Task.FromResult(new DiffResult(path, false, Array.Empty<DiffHunk>(), Array.Empty<AssetPropertyDelta>()));

    public Task<AssetMetadata> GetAssetInfoAsync(string path, CancellationToken ct = default)
        => Task.FromException<AssetMetadata>(new InvalidOperationException(NoRepo));

    public Task StageAsync(IEnumerable<string> paths, CancellationToken ct = default)      => throw new InvalidOperationException(NoRepo);
    public Task UnstageAsync(IEnumerable<string> paths, CancellationToken ct = default)    => throw new InvalidOperationException(NoRepo);
    public Task<CommitResult> CommitAsync(string message, CancellationToken ct = default)  => throw new InvalidOperationException(NoRepo);
    public Task PushAsync(IProgress<PushProgress>? progress = null, CancellationToken ct = default) => throw new InvalidOperationException(NoRepo);
    public Task PullAsync(IProgress<PullProgress>? progress = null, CancellationToken ct = default) => throw new InvalidOperationException(NoRepo);
    public Task LockAsync(string path, string? reason, CancellationToken ct = default)     => throw new InvalidOperationException(NoRepo);
    public Task UnlockAsync(string path, bool force, CancellationToken ct = default)       => throw new InvalidOperationException(NoRepo);
    public Task SwitchBranchAsync(string name, bool create, CancellationToken ct = default) => throw new InvalidOperationException(NoRepo);
    public Task CreateBranchAsync(string name, CancellationToken ct = default)              => throw new InvalidOperationException(NoRepo);
    public Task DeleteBranchAsync(string name, bool force, CancellationToken ct = default)  => throw new InvalidOperationException(NoRepo);
    public Task<CloneResult> CloneAsync(string url, string dest, IProgress<CloneProgress>? progress = null, CancellationToken ct = default) => throw new InvalidOperationException(NoRepo);
    public Task LoginAsync(string serverUrl, string user, string token, CancellationToken ct = default) => throw new InvalidOperationException(NoRepo);

#pragma warning disable CS1998
    public async IAsyncEnumerable<LockEvent> SubscribeLockEventsAsync([EnumeratorCancellation] CancellationToken ct = default)
    {
        yield break;
    }
#pragma warning restore CS1998
}
