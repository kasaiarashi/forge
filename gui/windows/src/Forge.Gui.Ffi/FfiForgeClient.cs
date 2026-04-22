using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Threading.Channels;
using Forge.Gui.Core.Models;
using Forge.Gui.Core.Services;

namespace Forge.Gui.Ffi;

/// <summary>
/// Real <see cref="IForgeClient"/> over the forge_ffi.dll C ABI.
/// Thread-safe at the operation granularity — the underlying Rust
/// session owns a tokio runtime and handles its own locking. Multiple
/// C# async ops can share a session.
/// </summary>
public sealed class FfiForgeClient : IForgeClient, IDisposable
{
    private ForgeSession? _session;
    private string? _workspacePath;

    private static readonly JsonSerializerOptions Json = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        PropertyNameCaseInsensitive = true,
    };

    public bool IsOpen => _session is { IsInvalid: false };

    public string? WorkspacePath => _workspacePath;

    public async Task<WorkspaceInfo> OpenAsync(string path, CancellationToken ct = default)
    {
        return await Task.Run(() =>
        {
            _session?.Dispose();
            _session = ForgeSession.Open(path);
            _workspacePath = path;

            // Start the lock-event stream so SubscribeLockEventsAsync
            // has events to drain. Idempotent on the Rust side.
            var err = default(NativeMethods.ForgeError);
            _ = NativeMethods.ForgeSubscribeLockEvents(_session.Raw, ref err);
            _ = NativeMethods.TakeError(ref err); // best-effort

            return FetchWorkspaceInfo();
        }, ct).ConfigureAwait(false);
    }

    private WorkspaceInfo FetchWorkspaceInfo()
    {
        var s = RequireSession();
        var err = default(NativeMethods.ForgeError);
        var ptr = NativeMethods.ForgeWorkspaceInfoJson(s.Raw, ref err);
        ThrowIfError(ref err);
        var json = NativeMethods.TakeString(ptr) ?? "{}";
        var dto = JsonSerializer.Deserialize<WorkspaceInfoDto>(json, Json)
                  ?? new WorkspaceInfoDto();
        var branch = dto.Head?.Kind == "branch" ? dto.Head.Name ?? "" : "(detached)";
        return new WorkspaceInfo(
            Path: dto.WorkspaceRoot,
            RepoName: string.IsNullOrEmpty(dto.Repo) ? SafeDirName(dto.WorkspaceRoot) : dto.Repo,
            ServerUrl: dto.RemoteUrl,
            CurrentBranch: branch,
            DefaultRemote: null);
    }

    public async Task<StatusResult> GetStatusAsync(CancellationToken ct = default)
    {
        return await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var ptr = NativeMethods.ForgeStatusJson(s.Raw, ref err);
            ThrowIfError(ref err);
            var json = NativeMethods.TakeString(ptr) ?? "{}";
            var dto = JsonSerializer.Deserialize<StatusJsonDto>(json, Json)
                      ?? new StatusJsonDto();
            var lockedLookup = dto.Locked.ToDictionary(l => l.Path, l => l.Owner, StringComparer.OrdinalIgnoreCase);
            var changes = new List<FileChange>();
            foreach (var p in dto.StagedNew)       changes.Add(MakeChange(p, FileChangeKind.StagedNew,      lockedLookup));
            foreach (var p in dto.StagedModified)  changes.Add(MakeChange(p, FileChangeKind.StagedModified, lockedLookup));
            foreach (var p in dto.StagedDeleted)   changes.Add(MakeChange(p, FileChangeKind.StagedDeleted,  lockedLookup));
            foreach (var p in dto.Modified)        changes.Add(MakeChange(p, FileChangeKind.Modified,       lockedLookup));
            foreach (var p in dto.Deleted)         changes.Add(MakeChange(p, FileChangeKind.Deleted,        lockedLookup));
            foreach (var p in dto.Untracked)       changes.Add(MakeChange(p, FileChangeKind.Untracked,      lockedLookup));
            return new StatusResult(changes, dto.Ahead, dto.Behind, HasConflicts: false);
        }, ct).ConfigureAwait(false);
    }

    public async Task<IReadOnlyList<LogEntry>> GetLogAsync(int limit, string? branch, CancellationToken ct = default)
    {
        return await Task.Run<IReadOnlyList<LogEntry>>(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var ptr = NativeMethods.ForgeLogJson(s.Raw, (uint)Math.Max(0, limit), ref err);
            ThrowIfError(ref err);
            var json = NativeMethods.TakeString(ptr) ?? "[]";
            var dtos = JsonSerializer.Deserialize<List<LogEntryDto>>(json, Json) ?? new();
            return dtos.Select(d => new LogEntry(
                Hash: d.Hash,
                ParentHashes: d.Parents,
                AuthorName: d.Author.Name,
                AuthorEmail: d.Author.Email,
                Timestamp: DateTimeOffset.TryParse(d.Timestamp, out var ts) ? ts : DateTimeOffset.UtcNow,
                Message: d.Message)).ToList();
        }, ct).ConfigureAwait(false);
    }

    public async Task<IReadOnlyList<Branch>> GetBranchesAsync(bool includeRemote, CancellationToken ct = default)
    {
        return await Task.Run<IReadOnlyList<Branch>>(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var ptr = NativeMethods.ForgeBranchListJson(s.Raw, ref err);
            ThrowIfError(ref err);
            var json = NativeMethods.TakeString(ptr) ?? "[]";
            var dtos = JsonSerializer.Deserialize<List<BranchDto>>(json, Json) ?? new();
            return dtos
                .Where(d => includeRemote || !d.IsRemote)
                .Select(d => new Branch(
                    Name: d.Name,
                    IsCurrent: d.IsCurrent,
                    IsRemote: d.IsRemote,
                    TipHash: d.TipHash,
                    AheadOfUpstream: null,
                    BehindUpstream: null))
                .ToList();
        }, ct).ConfigureAwait(false);
    }

    public async Task<IReadOnlyList<LockInfo>> GetLocksAsync(CancellationToken ct = default)
    {
        return await Task.Run<IReadOnlyList<LockInfo>>(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var ptr = NativeMethods.ForgeLockListJson(s.Raw, ref err);
            ThrowIfError(ref err);
            var json = NativeMethods.TakeString(ptr) ?? "[]";
            var dtos = JsonSerializer.Deserialize<List<LockDto>>(json, Json) ?? new();
            return dtos.Select(DtoToLock).ToList();
        }, ct).ConfigureAwait(false);
    }

    public Task<DiffResult> GetDiffAsync(string path, DiffKind kind, CancellationToken ct = default)
    {
        // FFI diff not shipped yet — caller falls through to an empty
        // placeholder so the DiffView can still render metadata.
        return Task.FromResult(new DiffResult(
            Path: path,
            IsBinary: path.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase)
                   || path.EndsWith(".umap", StringComparison.OrdinalIgnoreCase),
            Hunks: Array.Empty<DiffHunk>(),
            AssetDeltas: Array.Empty<AssetPropertyDelta>()));
    }

    public async Task<AssetMetadata> GetAssetInfoAsync(string path, CancellationToken ct = default)
    {
        return await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var ptr = NativeMethods.ForgeAssetInfoJson(s.Raw, path, ref err);
            ThrowIfError(ref err);
            var json = NativeMethods.TakeString(ptr) ?? "{}";
            var dto = JsonSerializer.Deserialize<AssetInfoDto>(json, Json) ?? new AssetInfoDto();
            return new AssetMetadata(
                Path: dto.Path,
                AssetClass: dto.AssetClass,
                EngineVersion: dto.EngineVersion,
                FileSize: dto.FileSize,
                Dependencies: dto.Dependencies);
        }, ct).ConfigureAwait(false);
    }

    public async Task StageAsync(IEnumerable<string> paths, CancellationToken ct = default)
    {
        var payload = JsonSerializer.Serialize(paths);
        await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var rc = NativeMethods.ForgeAddPaths(s.Raw, payload, ref err);
            if (rc != 0) ThrowIfError(ref err);
        }, ct).ConfigureAwait(false);
    }

    public async Task UnstageAsync(IEnumerable<string> paths, CancellationToken ct = default)
    {
        var payload = JsonSerializer.Serialize(paths);
        await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var rc = NativeMethods.ForgeUnstage(s.Raw, payload, ref err);
            if (rc != 0) ThrowIfError(ref err);
        }, ct).ConfigureAwait(false);
    }

    public async Task<CommitResult> CommitAsync(string message, CancellationToken ct = default)
    {
        await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var rc = NativeMethods.ForgeCommit(s.Raw, message, ref err);
            if (rc != 0) ThrowIfError(ref err);
        }, ct).ConfigureAwait(false);
        // forge_commit doesn't return the hash — caller can read it
        // from the next log fetch if needed.
        return new CommitResult(Hash: "", Message: message);
    }

    public async Task PushAsync(IProgress<PushProgress>? progress = null, CancellationToken ct = default)
    {
        progress?.Report(new PushProgress("starting", 0, 1));
        await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var rc = NativeMethods.ForgePush(s.Raw, 0, ref err);
            if (rc != 0) ThrowIfError(ref err);
        }, ct).ConfigureAwait(false);
        progress?.Report(new PushProgress("done", 1, 1));
    }

    public async Task PullAsync(IProgress<PullProgress>? progress = null, CancellationToken ct = default)
    {
        progress?.Report(new PullProgress("starting", 0, 1));
        await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var rc = NativeMethods.ForgePull(s.Raw, ref err);
            if (rc != 0) ThrowIfError(ref err);
        }, ct).ConfigureAwait(false);
        progress?.Report(new PullProgress("done", 1, 1));
    }

    public async Task LockAsync(string path, string? reason, CancellationToken ct = default)
    {
        await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var rc = NativeMethods.ForgeLockAcquire(s.Raw, path, reason, ref err);
            if (rc != 0) ThrowIfError(ref err);
        }, ct).ConfigureAwait(false);
    }

    public async Task UnlockAsync(string path, bool force, CancellationToken ct = default)
    {
        await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var rc = NativeMethods.ForgeLockRelease(s.Raw, path, ref err);
            if (rc != 0) ThrowIfError(ref err);
        }, ct).ConfigureAwait(false);
    }

    public async Task SwitchBranchAsync(string name, bool create, CancellationToken ct = default)
    {
        await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var rc = NativeMethods.ForgeSwitch(s.Raw, name, create ? 1 : 0, ref err);
            if (rc != 0) ThrowIfError(ref err);
        }, ct).ConfigureAwait(false);
    }

    public async Task CreateBranchAsync(string name, CancellationToken ct = default)
    {
        await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var rc = NativeMethods.ForgeBranchCreate(s.Raw, name, ref err);
            if (rc != 0) ThrowIfError(ref err);
        }, ct).ConfigureAwait(false);
    }

    public async Task DeleteBranchAsync(string name, bool force, CancellationToken ct = default)
    {
        await Task.Run(() =>
        {
            var s = RequireSession();
            var err = default(NativeMethods.ForgeError);
            var rc = NativeMethods.ForgeBranchDelete(s.Raw, name, force ? 1 : 0, ref err);
            if (rc != 0) ThrowIfError(ref err);
        }, ct).ConfigureAwait(false);
    }

    public Task<CloneResult> CloneAsync(string url, string dest, IProgress<CloneProgress>? progress = null, CancellationToken ct = default)
        => throw new NotImplementedException("forge_clone FFI not shipped yet — P3 milestone.");

    public Task LoginAsync(string serverUrl, string user, string token, CancellationToken ct = default)
        => throw new NotImplementedException("forge_login FFI not shipped yet — P3 milestone.");

    public async IAsyncEnumerable<LockEvent> SubscribeLockEventsAsync([EnumeratorCancellation] CancellationToken ct = default)
    {
        var channel = Channel.CreateUnbounded<LockEvent>();
        _ = Task.Run(async () =>
        {
            while (!ct.IsCancellationRequested)
            {
                try
                {
                    var s = _session;
                    if (s is null || s.IsInvalid) { await Task.Delay(300, ct).ConfigureAwait(false); continue; }
                    var err = default(NativeMethods.ForgeError);
                    var ptr = NativeMethods.ForgePollLockEventsJson(s.Raw, ref err);
                    var _errMsg = NativeMethods.TakeError(ref err);
                    var json = NativeMethods.TakeString(ptr) ?? "[]";
                    var events = JsonSerializer.Deserialize<List<LockEventDto>>(json, Json) ?? new();
                    foreach (var e in events)
                    {
                        if (e.Info is null) continue;
                        var kind = e.Kind == "release" ? LockEventKind.Released : LockEventKind.Acquired;
                        await channel.Writer.WriteAsync(
                            new LockEvent(kind, DtoToLock(e.Info)), ct).ConfigureAwait(false);
                    }
                }
                catch { /* best effort; keep the loop alive */ }
                try { await Task.Delay(500, ct).ConfigureAwait(false); } catch { break; }
            }
            channel.Writer.TryComplete();
        }, ct);

        while (await channel.Reader.WaitToReadAsync(ct).ConfigureAwait(false))
            while (channel.Reader.TryRead(out var evt))
                yield return evt;
    }

    public void Dispose()
    {
        _session?.Dispose();
        _session = null;
    }

    // ── Plumbing ─────────────────────────────────────────────────────

    private ForgeSession RequireSession()
        => _session is { IsInvalid: false } s
            ? s
            : throw new InvalidOperationException("Call OpenAsync before other IForgeClient methods.");

    private static void ThrowIfError(ref NativeMethods.ForgeError err)
    {
        var msg = NativeMethods.TakeError(ref err);
        if (msg is null) return;
        throw new ForgeFfiException(err.Code, msg);
    }

    private static FileChange MakeChange(string path, FileChangeKind kind, Dictionary<string, string> locks)
    {
        var isLocked = locks.TryGetValue(path, out var owner);
        return new FileChange(path, kind, isLocked, owner);
    }

    private static LockInfo DtoToLock(LockDto d)
    {
        var when = d.CreatedAt.HasValue
            ? DateTimeOffset.FromUnixTimeSeconds(d.CreatedAt.Value)
            : DateTimeOffset.UtcNow;
        return new LockInfo(d.Path, d.Owner, when, d.Reason);
    }

    private static string SafeDirName(string path)
    {
        try { return Path.GetFileName(Path.TrimEndingDirectorySeparator(path)); }
        catch { return path; }
    }
}
