namespace Forge.Gui.Core.Models;

public sealed record LockInfo(
    string Path,
    string Owner,
    DateTimeOffset AcquiredAt,
    string? Reason);

public enum LockEventKind { Acquired, Released }

public sealed record LockEvent(LockEventKind Kind, LockInfo Lock);
