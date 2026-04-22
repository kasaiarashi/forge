namespace Forge.Gui.Core.Models;

public enum DiffKind { Working, Staged, Commit }

public sealed record DiffHunk(int OldStart, int OldLen, int NewStart, int NewLen, string Body);

public sealed record AssetPropertyDelta(string PropertyPath, string? OldValue, string? NewValue);

public sealed record AssetMetadata(
    string Path,
    string AssetClass,
    string EngineVersion,
    long FileSize,
    IReadOnlyList<string> Dependencies);

public sealed record DiffResult(
    string Path,
    bool IsBinary,
    IReadOnlyList<DiffHunk> Hunks,
    IReadOnlyList<AssetPropertyDelta> AssetDeltas);
