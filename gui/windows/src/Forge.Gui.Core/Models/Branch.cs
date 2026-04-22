namespace Forge.Gui.Core.Models;

public sealed record Branch(
    string Name,
    bool IsCurrent,
    bool IsRemote,
    string? TipHash,
    int? AheadOfUpstream,
    int? BehindUpstream);
