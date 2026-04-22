using System.Text.Json.Serialization;

namespace Forge.Gui.Ffi;

// These records mirror the JSON shapes emitted by forge-ffi's *_json
// functions. They stay internal so callers see only the public
// Forge.Gui.Core.Models types, which FfiForgeClient maps onto.

internal sealed class StatusJsonDto
{
    [JsonPropertyName("workspace_root")] public string WorkspaceRoot { get; set; } = "";
    [JsonPropertyName("branch")] public string? Branch { get; set; }
    [JsonPropertyName("staged_new")] public List<string> StagedNew { get; set; } = new();
    [JsonPropertyName("staged_modified")] public List<string> StagedModified { get; set; } = new();
    [JsonPropertyName("staged_deleted")] public List<string> StagedDeleted { get; set; } = new();
    [JsonPropertyName("modified")] public List<string> Modified { get; set; } = new();
    [JsonPropertyName("deleted")] public List<string> Deleted { get; set; } = new();
    [JsonPropertyName("untracked")] public List<string> Untracked { get; set; } = new();
    [JsonPropertyName("locked")] public List<LockDto> Locked { get; set; } = new();
    [JsonPropertyName("ahead")] public int Ahead { get; set; }
    [JsonPropertyName("behind")] public int Behind { get; set; }
    [JsonPropertyName("remote")] public string? Remote { get; set; }
}

internal sealed class LockDto
{
    [JsonPropertyName("path")] public string Path { get; set; } = "";
    [JsonPropertyName("owner")] public string Owner { get; set; } = "";
    [JsonPropertyName("workspace_id")] public string? WorkspaceId { get; set; }
    [JsonPropertyName("reason")] public string? Reason { get; set; }
    [JsonPropertyName("created_at")] public long? CreatedAt { get; set; }
}

internal sealed class LogEntryDto
{
    [JsonPropertyName("hash")] public string Hash { get; set; } = "";
    [JsonPropertyName("short_hash")] public string ShortHash { get; set; } = "";
    [JsonPropertyName("parents")] public List<string> Parents { get; set; } = new();
    [JsonPropertyName("author")] public AuthorDto Author { get; set; } = new();
    [JsonPropertyName("timestamp")] public string Timestamp { get; set; } = "";
    [JsonPropertyName("message")] public string Message { get; set; } = "";
}

internal sealed class AuthorDto
{
    [JsonPropertyName("name")] public string Name { get; set; } = "";
    [JsonPropertyName("email")] public string Email { get; set; } = "";
}

internal sealed class BranchDto
{
    [JsonPropertyName("name")] public string Name { get; set; } = "";
    [JsonPropertyName("is_current")] public bool IsCurrent { get; set; }
    [JsonPropertyName("is_remote")] public bool IsRemote { get; set; }
    [JsonPropertyName("tip_hash")] public string? TipHash { get; set; }
}

internal sealed class WorkspaceInfoDto
{
    [JsonPropertyName("workspace_root")] public string WorkspaceRoot { get; set; } = "";
    [JsonPropertyName("workspace_id")] public string? WorkspaceId { get; set; }
    [JsonPropertyName("repo")] public string Repo { get; set; } = "";
    [JsonPropertyName("remote_url")] public string? RemoteUrl { get; set; }
    [JsonPropertyName("head")] public HeadDto? Head { get; set; }
    [JsonPropertyName("user")] public AuthorDto? User { get; set; }
}

internal sealed class HeadDto
{
    [JsonPropertyName("kind")] public string Kind { get; set; } = "";
    [JsonPropertyName("name")] public string? Name { get; set; }
    [JsonPropertyName("hash")] public string? Hash { get; set; }
}

internal sealed class AssetInfoDto
{
    [JsonPropertyName("path")] public string Path { get; set; } = "";
    [JsonPropertyName("asset_class")] public string AssetClass { get; set; } = "";
    [JsonPropertyName("engine_version")] public string EngineVersion { get; set; } = "";
    [JsonPropertyName("package_flags")] public List<string> PackageFlags { get; set; } = new();
    [JsonPropertyName("dependencies")] public List<string> Dependencies { get; set; } = new();
    [JsonPropertyName("file_size")] public long FileSize { get; set; }
}

internal sealed class LockEventDto
{
    [JsonPropertyName("kind")] public string Kind { get; set; } = "";
    [JsonPropertyName("seq")] public long Seq { get; set; }
    [JsonPropertyName("info")] public LockDto? Info { get; set; }
}
