namespace Forge.Gui.Core.Models;

public sealed record PushProgress(string Stage, long Current, long Total);
public sealed record PullProgress(string Stage, long Current, long Total);
public sealed record CloneProgress(string Stage, long Current, long Total);

public sealed record CommitResult(string Hash, string Message);
public sealed record CloneResult(string Path, string RepoName);
