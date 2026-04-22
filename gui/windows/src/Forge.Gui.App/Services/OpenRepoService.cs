using Forge.Gui.Core.Models;
using Forge.Gui.Core.Services;
using Forge.Gui.Ffi;

namespace Forge.Gui.App.Services;

/// <summary>
/// Bridges the folder picker to the FFI session. On success swaps
/// <see cref="ActiveForgeClient.Inner"/> to a new <see cref="FfiForgeClient"/>
/// and updates <see cref="IRepoRegistry"/>. On close the inner falls
/// back to <see cref="NoWorkspaceClient"/> so view-models see an
/// empty state instead of stale data.
/// </summary>
public sealed class OpenRepoService
{
    private readonly ActiveForgeClient _active;
    private readonly IRepoRegistry _registry;
    private readonly ISettingsStore _settings;
    private readonly NoWorkspaceClient _fallback;

    public event EventHandler<WorkspaceInfo>? WorkspaceOpened;

    public OpenRepoService(
        ActiveForgeClient active,
        IRepoRegistry registry,
        ISettingsStore settings,
        NoWorkspaceClient fallback)
    {
        _active = active;
        _registry = registry;
        _settings = settings;
        _fallback = fallback;
    }

    public async Task<WorkspaceInfo?> OpenAsync(string workspacePath, CancellationToken ct = default)
    {
        Diag($"--- OpenAsync ---");
        Diag($"input='{workspacePath}'");

        // Normalise before handing off to Rust: collapse .. / trailing
        // separators and resolve to an absolute path. FolderPicker on
        // Win11 sometimes hands back `\\?\`-prefixed long paths, which
        // trip Workspace::discover's fs walk if passed unchanged.
        var normalized = Path.GetFullPath(workspacePath).TrimEnd(Path.DirectorySeparatorChar);
        if (normalized.StartsWith(@"\\?\", StringComparison.Ordinal))
            normalized = normalized.Substring(4);
        Diag($"normalized='{normalized}'");

        if (!Directory.Exists(normalized))
        {
            Diag("Directory.Exists=false");
            throw new DirectoryNotFoundException($"Folder does not exist: {normalized}");
        }

        // Pre-flight .forge/ check so we can give a user-friendly
        // error before paying the FFI round-trip. Workspace::discover
        // also walks up from the chosen dir, so if a parent has .forge/
        // we still let the FFI succeed — that matches CLI behaviour.
        var ancestor = FindForgeAncestor(normalized);
        Diag(ancestor is null ? "no .forge found walking up" : $"found .forge at '{ancestor}'");
        if (ancestor is null)
            throw new ForgeFfiException(
                ForgeErrorCodes.NotAWorkspace,
                $"No .forge directory found in '{normalized}' or any parent. " +
                "Pick the folder that contains .forge/ (typically your project root).");

        var client = new FfiForgeClient();
        try
        {
            var info = await client.OpenAsync(normalized, ct).ConfigureAwait(false);
            Diag($"forge_session_open OK, branch='{info.CurrentBranch}', root='{info.Path}'");

            // Dispose any previous FFI session before the swap so the
            // old tokio runtime + file handles release cleanly.
            if (_active.Inner is FfiForgeClient oldFfi && !ReferenceEquals(oldFfi, client))
                oldFfi.Dispose();

            _active.Inner = client;

            var display = string.IsNullOrEmpty(info.RepoName) ? SafeName(info.Path) : info.RepoName;
            var entry = new RepoEntry(info.Path, display, AccentHex: "#5EB8FF");
            _registry.SetActive(entry);

            await _settings.UpdateAsync(s =>
            {
                s.LastActiveRepoPath = info.Path;
                var existing = s.RecentRepos.FirstOrDefault(r =>
                    string.Equals(r.Path, info.Path, StringComparison.OrdinalIgnoreCase));
                if (existing is not null) s.RecentRepos.Remove(existing);
                s.RecentRepos.Insert(0, new PersistedRepo
                {
                    Path = info.Path,
                    DisplayName = display,
                    LastOpened = DateTimeOffset.UtcNow,
                });
                if (s.RecentRepos.Count > 12)
                    s.RecentRepos.RemoveRange(12, s.RecentRepos.Count - 12);
            }, ct).ConfigureAwait(false);

            WorkspaceOpened?.Invoke(this, info);
            return info;
        }
        catch (Exception ex)
        {
            Diag($"FFI failed: [{ex.GetType().Name}] code={(ex as ForgeFfiException)?.Code} msg={ex.Message}");
            client.Dispose();
            throw;
        }
    }

    public void CloseCurrent()
    {
        if (_active.Inner is FfiForgeClient ffi)
        {
            ffi.Dispose();
            _active.Inner = _fallback;
        }
    }

    public bool IsWorkspaceOpen => _active.Inner is FfiForgeClient;

    private static string SafeName(string path)
    {
        try { return Path.GetFileName(Path.TrimEndingDirectorySeparator(path)); }
        catch { return path; }
    }

    /// <summary>
    /// Walks up from <paramref name="start"/> looking for a <c>.forge</c>
    /// directory. Returns the workspace root (directory containing
    /// <c>.forge</c>) or <c>null</c> if none found. Mirrors
    /// <c>Workspace::discover</c> in forge-core.
    /// </summary>
    private static string? FindForgeAncestor(string start)
    {
        var dir = new DirectoryInfo(start);
        while (dir is not null)
        {
            if (Directory.Exists(Path.Combine(dir.FullName, ".forge")))
                return dir.FullName;
            dir = dir.Parent;
        }
        return null;
    }

    private static readonly object DiagLock = new();
    private static readonly string DiagPath = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "ForgeVCS", "open-repo-diag.log");

    private static void Diag(string msg) => DiagLog(msg);

    /// <summary>
    /// Append-only diagnostic log to
    /// <c>%LocalAppData%\ForgeVCS\open-repo-diag.log</c>. Called from
    /// UI + background threads during the open flow so we can attribute
    /// a crash to a specific step without attaching a debugger.
    /// </summary>
    public static void DiagLog(string msg)
    {
        try
        {
            lock (DiagLock)
            {
                Directory.CreateDirectory(Path.GetDirectoryName(DiagPath)!);
                File.AppendAllText(DiagPath, $"[{DateTimeOffset.Now:HH:mm:ss.fff}] {msg}\n");
            }
        }
        catch { /* never let diagnostics break the open path */ }
    }
}
