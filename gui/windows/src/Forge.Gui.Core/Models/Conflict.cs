namespace Forge.Gui.Core.Models;

public enum ConflictChoice { TakeOurs, TakeTheirs, Custom, Unresolved }

public sealed record PropertyConflict(
    string PropertyPath,
    string? BaseValue,
    string? OurValue,
    string? TheirValue,
    ConflictChoice Choice,
    string? CustomValue);

public sealed record FileConflict(
    string Path,
    bool IsBinary,
    IReadOnlyList<PropertyConflict> Properties);
