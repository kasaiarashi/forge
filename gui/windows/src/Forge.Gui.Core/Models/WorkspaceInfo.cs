namespace Forge.Gui.Core.Models;

public sealed record WorkspaceInfo(
    string Path,
    string RepoName,
    string? ServerUrl,
    string CurrentBranch,
    string? DefaultRemote);
