using Forge.Gui.Core.Models;

namespace Forge.Gui.Core.Services;

/// <summary>
/// Swappable <see cref="IForgeClient"/> proxy. The App registers
/// ONE instance of this in DI; at runtime we flip the underlying
/// <see cref="Inner"/> between the mock (no repo open) and the real
/// FFI-backed client (repo open) without re-registering or blowing
/// away view-model subscriptions.
/// </summary>
public sealed class ActiveForgeClient : IForgeClient
{
    private IForgeClient _inner;

    public event EventHandler? InnerChanged;

    public IForgeClient Inner
    {
        get => _inner;
        set
        {
            if (ReferenceEquals(_inner, value)) return;
            _inner = value;
            InnerChanged?.Invoke(this, EventArgs.Empty);
        }
    }

    public ActiveForgeClient(IForgeClient initial)
    {
        _inner = initial;
    }

    public Task<WorkspaceInfo> OpenAsync(string path, CancellationToken ct = default)
        => _inner.OpenAsync(path, ct);

    public Task<StatusResult> GetStatusAsync(CancellationToken ct = default)
        => _inner.GetStatusAsync(ct);

    public Task<IReadOnlyList<LogEntry>> GetLogAsync(int limit, string? branch, CancellationToken ct = default)
        => _inner.GetLogAsync(limit, branch, ct);

    public Task<IReadOnlyList<Branch>> GetBranchesAsync(bool includeRemote, CancellationToken ct = default)
        => _inner.GetBranchesAsync(includeRemote, ct);

    public Task<IReadOnlyList<LockInfo>> GetLocksAsync(CancellationToken ct = default)
        => _inner.GetLocksAsync(ct);

    public Task<DiffResult> GetDiffAsync(string path, DiffKind kind, CancellationToken ct = default)
        => _inner.GetDiffAsync(path, kind, ct);

    public Task<AssetMetadata> GetAssetInfoAsync(string path, CancellationToken ct = default)
        => _inner.GetAssetInfoAsync(path, ct);

    public Task StageAsync(IEnumerable<string> paths, CancellationToken ct = default)
        => _inner.StageAsync(paths, ct);

    public Task UnstageAsync(IEnumerable<string> paths, CancellationToken ct = default)
        => _inner.UnstageAsync(paths, ct);

    public Task<CommitResult> CommitAsync(string message, CancellationToken ct = default)
        => _inner.CommitAsync(message, ct);

    public Task PushAsync(IProgress<PushProgress>? progress = null, CancellationToken ct = default)
        => _inner.PushAsync(progress, ct);

    public Task PullAsync(IProgress<PullProgress>? progress = null, CancellationToken ct = default)
        => _inner.PullAsync(progress, ct);

    public Task LockAsync(string path, string? reason, CancellationToken ct = default)
        => _inner.LockAsync(path, reason, ct);

    public Task UnlockAsync(string path, bool force, CancellationToken ct = default)
        => _inner.UnlockAsync(path, force, ct);

    public Task SwitchBranchAsync(string name, bool create, CancellationToken ct = default)
        => _inner.SwitchBranchAsync(name, create, ct);

    public Task CreateBranchAsync(string name, CancellationToken ct = default)
        => _inner.CreateBranchAsync(name, ct);

    public Task DeleteBranchAsync(string name, bool force, CancellationToken ct = default)
        => _inner.DeleteBranchAsync(name, force, ct);

    public Task<CloneResult> CloneAsync(string url, string dest, IProgress<CloneProgress>? progress = null, CancellationToken ct = default)
        => _inner.CloneAsync(url, dest, progress, ct);

    public Task LoginAsync(string serverUrl, string user, string token, CancellationToken ct = default)
        => _inner.LoginAsync(serverUrl, user, token, ct);

    public IAsyncEnumerable<LockEvent> SubscribeLockEventsAsync(CancellationToken ct = default)
        => _inner.SubscribeLockEventsAsync(ct);
}
