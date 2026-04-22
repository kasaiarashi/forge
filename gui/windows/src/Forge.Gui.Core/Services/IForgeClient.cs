using Forge.Gui.Core.Models;

namespace Forge.Gui.Core.Services;

public interface IForgeClient
{
    Task<WorkspaceInfo> OpenAsync(string path, CancellationToken ct = default);
    Task<StatusResult> GetStatusAsync(CancellationToken ct = default);
    Task<IReadOnlyList<LogEntry>> GetLogAsync(int limit, string? branch, CancellationToken ct = default);
    Task<IReadOnlyList<Branch>> GetBranchesAsync(bool includeRemote, CancellationToken ct = default);
    Task<IReadOnlyList<LockInfo>> GetLocksAsync(CancellationToken ct = default);
    Task<DiffResult> GetDiffAsync(string path, DiffKind kind, CancellationToken ct = default);
    Task<AssetMetadata> GetAssetInfoAsync(string path, CancellationToken ct = default);

    Task StageAsync(IEnumerable<string> paths, CancellationToken ct = default);
    Task UnstageAsync(IEnumerable<string> paths, CancellationToken ct = default);
    Task<CommitResult> CommitAsync(string message, CancellationToken ct = default);
    Task PushAsync(IProgress<PushProgress>? progress = null, CancellationToken ct = default);
    Task PullAsync(IProgress<PullProgress>? progress = null, CancellationToken ct = default);

    Task LockAsync(string path, string? reason, CancellationToken ct = default);
    Task UnlockAsync(string path, bool force, CancellationToken ct = default);

    Task SwitchBranchAsync(string name, bool create, CancellationToken ct = default);
    Task CreateBranchAsync(string name, CancellationToken ct = default);
    Task DeleteBranchAsync(string name, bool force, CancellationToken ct = default);

    Task<CloneResult> CloneAsync(string url, string dest, IProgress<CloneProgress>? progress = null, CancellationToken ct = default);
    Task LoginAsync(string serverUrl, string user, string token, CancellationToken ct = default);

    IAsyncEnumerable<LockEvent> SubscribeLockEventsAsync(CancellationToken ct = default);
}
