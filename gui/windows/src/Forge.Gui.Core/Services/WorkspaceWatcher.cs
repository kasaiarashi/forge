using System.Threading.Channels;

namespace Forge.Gui.Core.Services;

public sealed record WorkspaceChange(string Path, WatcherChangeTypes Kind);

public enum WatcherChangeTypes { Created, Changed, Deleted, Renamed }

public interface IWorkspaceWatcher : IDisposable
{
    IAsyncEnumerable<WorkspaceChange> Changes { get; }
    void Start(string workspaceRoot);
    void Stop();
}

/// <summary>
/// Debounced <see cref="FileSystemWatcher"/> wrapper. Ignores `.forge/`
/// and common transient files (.tmp, .swp, .~lock, `~$...`).
/// Coalesces bursts within <see cref="_debounce"/> into one change per path.
/// </summary>
public sealed class WorkspaceWatcher : IWorkspaceWatcher
{
    private readonly TimeSpan _debounce;
    private readonly Channel<WorkspaceChange> _channel = Channel.CreateUnbounded<WorkspaceChange>();
    private readonly Dictionary<string, (WatcherChangeTypes Kind, DateTimeOffset At)> _pending = new(StringComparer.OrdinalIgnoreCase);
    private readonly object _lock = new();
    private FileSystemWatcher? _fsw;
    private CancellationTokenSource? _flushCts;

    public IAsyncEnumerable<WorkspaceChange> Changes => _channel.Reader.ReadAllAsync();

    public WorkspaceWatcher(TimeSpan? debounce = null)
    {
        _debounce = debounce ?? TimeSpan.FromMilliseconds(200);
    }

    public void Start(string workspaceRoot)
    {
        Stop();
        _fsw = new FileSystemWatcher(workspaceRoot)
        {
            IncludeSubdirectories = true,
            NotifyFilter = NotifyFilters.FileName | NotifyFilters.DirectoryName
                         | NotifyFilters.LastWrite | NotifyFilters.Size,
            InternalBufferSize = 64 * 1024,
        };
        _fsw.Created += (_, e) => Enqueue(e.FullPath, WatcherChangeTypes.Created);
        _fsw.Changed += (_, e) => Enqueue(e.FullPath, WatcherChangeTypes.Changed);
        _fsw.Deleted += (_, e) => Enqueue(e.FullPath, WatcherChangeTypes.Deleted);
        _fsw.Renamed += (_, e) => Enqueue(e.FullPath, WatcherChangeTypes.Renamed);
        _fsw.EnableRaisingEvents = true;

        _flushCts = new CancellationTokenSource();
        _ = FlushLoopAsync(_flushCts.Token);
    }

    public void Stop()
    {
        _flushCts?.Cancel();
        _flushCts = null;
        if (_fsw is not null)
        {
            _fsw.EnableRaisingEvents = false;
            _fsw.Dispose();
            _fsw = null;
        }
        lock (_lock) _pending.Clear();
    }

    private void Enqueue(string fullPath, WatcherChangeTypes kind)
    {
        if (IsIgnored(fullPath)) return;
        lock (_lock)
        {
            _pending[fullPath] = (kind, DateTimeOffset.UtcNow);
        }
    }

    private async Task FlushLoopAsync(CancellationToken ct)
    {
        try
        {
            while (!ct.IsCancellationRequested)
            {
                await Task.Delay(_debounce, ct).ConfigureAwait(false);
                List<WorkspaceChange>? ready = null;
                var cutoff = DateTimeOffset.UtcNow - _debounce;
                lock (_lock)
                {
                    foreach (var kv in _pending)
                    {
                        if (kv.Value.At <= cutoff)
                            (ready ??= new()).Add(new WorkspaceChange(kv.Key, kv.Value.Kind));
                    }
                    if (ready is not null)
                        foreach (var r in ready)
                            _pending.Remove(r.Path);
                }
                if (ready is null) continue;
                foreach (var change in ready)
                    await _channel.Writer.WriteAsync(change, ct).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException) { /* Stop */ }
    }

    private static bool IsIgnored(string fullPath)
    {
        var name = Path.GetFileName(fullPath);
        if (string.IsNullOrEmpty(name)) return false;
        if (name.StartsWith("~$", StringComparison.Ordinal)) return true;
        if (name.StartsWith(".~lock", StringComparison.Ordinal)) return true;
        if (name.EndsWith(".tmp", StringComparison.OrdinalIgnoreCase)) return true;
        if (name.EndsWith(".swp", StringComparison.OrdinalIgnoreCase)) return true;

        // Exclude anything under a `.forge/` segment.
        var segments = fullPath.Split(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
        foreach (var s in segments)
            if (string.Equals(s, ".forge", StringComparison.OrdinalIgnoreCase))
                return true;
        return false;
    }

    public void Dispose()
    {
        Stop();
        _channel.Writer.TryComplete();
    }
}
