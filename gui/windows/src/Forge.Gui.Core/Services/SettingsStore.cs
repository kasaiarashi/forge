using System.Text.Json;
using System.Text.Json.Serialization;

namespace Forge.Gui.Core.Services;

public sealed class AppSettings
{
    public UiMode Mode { get; set; } = UiMode.Simple;
    public AppTheme Theme { get; set; } = AppTheme.Dark;
    public string? LastActiveRepoPath { get; set; }
    public List<PersistedRepo> RecentRepos { get; set; } = new();
    public string AccentHex { get; set; } = "#5EB8FF";
    public bool CompactDensity { get; set; }
    public string? ExternalMergeToolPath { get; set; }
    public string? UnrealEditorPath { get; set; }
}

public sealed class PersistedRepo
{
    public required string Path { get; set; }
    public required string DisplayName { get; set; }
    public string AccentHex { get; set; } = "#5EB8FF";
    public DateTimeOffset LastOpened { get; set; } = DateTimeOffset.UtcNow;
}

[JsonConverter(typeof(JsonStringEnumConverter))]
public enum AppTheme { System, Light, Dark }

public interface ISettingsStore
{
    AppSettings Current { get; }
    event EventHandler? Changed;
    Task LoadAsync(CancellationToken ct = default);
    Task SaveAsync(CancellationToken ct = default);
    Task UpdateAsync(Action<AppSettings> mutate, CancellationToken ct = default);
}

public sealed class JsonSettingsStore : ISettingsStore
{
    private readonly string _path;
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private AppSettings _current = new();

    private static readonly JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };

    public AppSettings Current => _current;
    public event EventHandler? Changed;

    public JsonSettingsStore(string? pathOverride = null)
    {
        _path = pathOverride ?? DefaultPath();
    }

    public static string DefaultPath()
    {
        var local = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        return Path.Combine(local, "ForgeVCS", "gui-settings.json");
    }

    public async Task LoadAsync(CancellationToken ct = default)
    {
        if (!File.Exists(_path)) return;
        try
        {
            await using var fs = File.OpenRead(_path);
            var loaded = await JsonSerializer.DeserializeAsync<AppSettings>(fs, Json, ct).ConfigureAwait(false);
            if (loaded is not null)
            {
                _current = loaded;
                Changed?.Invoke(this, EventArgs.Empty);
            }
        }
        catch (JsonException)
        {
            // Malformed file — keep defaults; don't delete so user can inspect.
        }
    }

    public async Task SaveAsync(CancellationToken ct = default)
    {
        await _writeLock.WaitAsync(ct).ConfigureAwait(false);
        try
        {
            var dir = Path.GetDirectoryName(_path)!;
            Directory.CreateDirectory(dir);
            var tmp = _path + ".tmp";
            await using (var fs = File.Create(tmp))
                await JsonSerializer.SerializeAsync(fs, _current, Json, ct).ConfigureAwait(false);
            File.Move(tmp, _path, overwrite: true);
        }
        finally { _writeLock.Release(); }
    }

    public async Task UpdateAsync(Action<AppSettings> mutate, CancellationToken ct = default)
    {
        mutate(_current);
        Changed?.Invoke(this, EventArgs.Empty);
        await SaveAsync(ct).ConfigureAwait(false);
    }
}
