namespace Forge.Gui.Core.Models;

public enum FileChangeKind
{
    Untracked,
    Modified,
    Deleted,
    StagedNew,
    StagedModified,
    StagedDeleted,
    Conflicted,
}

public sealed record FileChange(string Path, FileChangeKind Kind, bool IsLocked, string? LockOwner);

public sealed record StatusResult(
    IReadOnlyList<FileChange> Changes,
    int Ahead,
    int Behind,
    bool HasConflicts);
