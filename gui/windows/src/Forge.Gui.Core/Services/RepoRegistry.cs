namespace Forge.Gui.Core.Services;

public sealed record RepoEntry(string Path, string DisplayName, string AccentHex);

public interface IRepoRegistry
{
    IReadOnlyList<RepoEntry> Recent { get; }
    RepoEntry? Active { get; }
    event EventHandler<RepoEntry?>? ActiveChanged;
    void Add(RepoEntry entry);
    void SetActive(RepoEntry entry);
}

public sealed class InMemoryRepoRegistry : IRepoRegistry
{
    private readonly List<RepoEntry> _recent = new();
    private RepoEntry? _active;

    public IReadOnlyList<RepoEntry> Recent => _recent;
    public RepoEntry? Active => _active;
    public event EventHandler<RepoEntry?>? ActiveChanged;

    public void Add(RepoEntry entry)
    {
        _recent.RemoveAll(r => string.Equals(r.Path, entry.Path, StringComparison.OrdinalIgnoreCase));
        _recent.Insert(0, entry);
    }

    public void SetActive(RepoEntry entry)
    {
        Add(entry);
        _active = entry;
        ActiveChanged?.Invoke(this, entry);
    }
}
