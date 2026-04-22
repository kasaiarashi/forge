using System.Runtime.InteropServices;

namespace Forge.Gui.Ffi;

/// <summary>
/// Direct P/Invoke bindings into forge_ffi.dll (the Rust cdylib under
/// crates/forge-ffi). All signatures mirror the C ABI exactly — no
/// marshalling cleverness. String returns are allocated on the Rust
/// side and must be released via <see cref="ForgeStringFree"/>.
/// </summary>
internal static partial class NativeMethods
{
    public const string Dll = "forge_ffi";

    // ── Status codes (forge_status_t) ────────────────────────────────
    public const int FORGE_OK = 0;
    public const int FORGE_ERR_IO = 1;
    public const int FORGE_ERR_ARG = 2;
    public const int FORGE_ERR_AUTH = 3;
    public const int FORGE_ERR_NOT_FOUND = 4;
    public const int FORGE_ERR_CONFLICT = 5;
    public const int FORGE_ERR_NOT_A_WORKSPACE = 6;
    public const int FORGE_ERR_INTERNAL = 99;

    [StructLayout(LayoutKind.Sequential)]
    public struct ForgeError
    {
        public int Code;
        public IntPtr Message;
    }

    // ── Core lifecycle ───────────────────────────────────────────────

    [LibraryImport(Dll, EntryPoint = "forge_version")]
    public static partial IntPtr ForgeVersion();

    [LibraryImport(Dll, EntryPoint = "forge_abi_version")]
    public static partial int ForgeAbiVersion();

    [LibraryImport(Dll, EntryPoint = "forge_string_free")]
    public static partial void ForgeStringFree(IntPtr s);

    [LibraryImport(Dll, EntryPoint = "forge_error_free")]
    public static partial void ForgeErrorFree(ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_session_open", StringMarshalling = StringMarshalling.Utf8)]
    public static partial IntPtr ForgeSessionOpen(string workspacePath, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_session_close")]
    public static partial void ForgeSessionClose(IntPtr session);

    // ── Read ops ─────────────────────────────────────────────────────

    [LibraryImport(Dll, EntryPoint = "forge_status_json")]
    public static partial IntPtr ForgeStatusJson(IntPtr session, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_workspace_info_json")]
    public static partial IntPtr ForgeWorkspaceInfoJson(IntPtr session, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_current_branch")]
    public static partial IntPtr ForgeCurrentBranch(IntPtr session, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_log_json")]
    public static partial IntPtr ForgeLogJson(IntPtr session, uint limit, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_branch_list_json")]
    public static partial IntPtr ForgeBranchListJson(IntPtr session, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_lock_list_json")]
    public static partial IntPtr ForgeLockListJson(IntPtr session, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_asset_info_json", StringMarshalling = StringMarshalling.Utf8)]
    public static partial IntPtr ForgeAssetInfoJson(IntPtr session, string path, ref ForgeError err);

    // ── Write ops ────────────────────────────────────────────────────

    [LibraryImport(Dll, EntryPoint = "forge_add_paths", StringMarshalling = StringMarshalling.Utf8)]
    public static partial int ForgeAddPaths(IntPtr session, string pathsJson, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_unstage", StringMarshalling = StringMarshalling.Utf8)]
    public static partial int ForgeUnstage(IntPtr session, string pathsJson, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_commit", StringMarshalling = StringMarshalling.Utf8)]
    public static partial int ForgeCommit(IntPtr session, string message, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_push")]
    public static partial int ForgePush(IntPtr session, int force, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_pull")]
    public static partial int ForgePull(IntPtr session, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_switch", StringMarshalling = StringMarshalling.Utf8)]
    public static partial int ForgeSwitch(IntPtr session, string name, int create, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_branch_create", StringMarshalling = StringMarshalling.Utf8)]
    public static partial int ForgeBranchCreate(IntPtr session, string name, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_branch_delete", StringMarshalling = StringMarshalling.Utf8)]
    public static partial int ForgeBranchDelete(IntPtr session, string name, int force, ref ForgeError err);

    // ── Locks ────────────────────────────────────────────────────────

    [LibraryImport(Dll, EntryPoint = "forge_lock_acquire", StringMarshalling = StringMarshalling.Utf8)]
    public static partial int ForgeLockAcquire(IntPtr session, string path, string? reason, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_lock_release", StringMarshalling = StringMarshalling.Utf8)]
    public static partial int ForgeLockRelease(IntPtr session, string path, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_subscribe_lock_events")]
    public static partial int ForgeSubscribeLockEvents(IntPtr session, ref ForgeError err);

    [LibraryImport(Dll, EntryPoint = "forge_poll_lock_events_json")]
    public static partial IntPtr ForgePollLockEventsJson(IntPtr session, ref ForgeError err);

    // ── Helpers ──────────────────────────────────────────────────────

    /// <summary>
    /// Takes ownership of a Rust-allocated <c>*mut c_char</c>, copies it
    /// to a managed string, and frees the original via
    /// <see cref="ForgeStringFree"/>. Returns <c>null</c> when <c>ptr</c>
    /// is zero.
    /// </summary>
    public static string? TakeString(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero) return null;
        try { return Marshal.PtrToStringUTF8(ptr); }
        finally { ForgeStringFree(ptr); }
    }

    /// <summary>
    /// Reads an error struct populated by the Rust side and frees its
    /// internal allocation. Returns <c>null</c> for <see cref="FORGE_OK"/>.
    /// </summary>
    public static string? TakeError(ref ForgeError err)
    {
        if (err.Code == FORGE_OK) return null;
        var msg = err.Message != IntPtr.Zero
            ? Marshal.PtrToStringUTF8(err.Message)
            : $"forge-ffi error (code={err.Code})";
        ForgeErrorFree(ref err);
        return msg ?? $"forge-ffi error (code={err.Code})";
    }
}
