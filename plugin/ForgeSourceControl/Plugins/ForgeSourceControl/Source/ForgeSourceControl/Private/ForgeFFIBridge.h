// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#pragma once

#include "CoreMinimal.h"

/**
 * C++ wrapper around `forge_ffi.dll` (Phase 4c.1).
 *
 * The plugin today fans out ~3N+2 subprocess calls per CheckIn — each
 * one pays Windows process-creation overhead (~15 ms cold, ~3 ms warm)
 * and blocks the game thread while stdout is drained. Loading the Rust
 * library in-process collapses that to one dynamic link at module init
 * and direct function calls thereafter; the 500-asset CheckIn target
 * for Phase 4 (< 2 s) is unreachable without this.
 *
 * This header is the bridge layer: it hides the raw `extern "C"`
 * signatures from the rest of the plugin behind RAII types and
 * UE-native conveniences (`FString` in/out, `TArray`, `FText` for
 * errors). Workers are expected to call through `FForgeFFI::Get()`,
 * not the C ABI directly.
 *
 * ## Module lifetime
 *
 * `FForgeFFI::Initialize` is called from `FForgeSourceControlModule::
 * StartupModule`. It loads the DLL via `FPlatformProcess::GetDllHandle`
 * and resolves the exported symbols. A failure to load is non-fatal —
 * every wrapper falls back to a "fbi disabled" error so the plugin can
 * keep working via the legacy subprocess path during the migration.
 *
 * `FForgeFFI::Shutdown` closes the DLL from `ShutdownModule`.
 */

// The cdylib's C header. Relative include path — the plugin's Build.cs
// (updated in 4c.1) adds `<repo>/crates/forge-ffi/include` to
// PublicIncludePaths. During the migration window, consumers that
// haven't built the Rust side first get the sentinel below.
#if __has_include("forge_ffi.h")
	#include "forge_ffi.h"
	#define FORGE_FFI_HAVE_HEADER 1
#else
	#define FORGE_FFI_HAVE_HEADER 0
	// Stand-in types so this header still compiles when the Rust
	// include dir isn't wired up yet. The .cpp uses `FORGE_FFI_HAVE_HEADER`
	// to gate the actual dlopen path.
	struct forge_session_t;
	struct forge_error_t;
	typedef int forge_status_t;
#endif

/**
 * Session handle. RAII-free type alias — ownership is expressed via
 * `FForgeFFISession` below so callers can't accidentally leak.
 */
using FForgeFFISessionPtr = forge_session_t*;

/**
 * RAII wrapper around `forge_session_t*`. Movable, not copyable.
 * Calls `forge_session_close` on scope exit.
 */
class FORGESOURCECONTROL_API FForgeFFISession
{
public:
	FForgeFFISession() = default;
	explicit FForgeFFISession(FForgeFFISessionPtr InRaw) : Raw(InRaw) {}

	FForgeFFISession(const FForgeFFISession&) = delete;
	FForgeFFISession& operator=(const FForgeFFISession&) = delete;

	FForgeFFISession(FForgeFFISession&& Other) noexcept : Raw(Other.Raw) { Other.Raw = nullptr; }
	FForgeFFISession& operator=(FForgeFFISession&& Other) noexcept
	{
		if (this != &Other)
		{
			Close();
			Raw = Other.Raw;
			Other.Raw = nullptr;
		}
		return *this;
	}

	~FForgeFFISession() { Close(); }

	bool IsValid() const { return Raw != nullptr; }
	FForgeFFISessionPtr Get() const { return Raw; }

	/** Release ownership without closing. Caller takes over the close. */
	FForgeFFISessionPtr Release()
	{
		FForgeFFISessionPtr R = Raw;
		Raw = nullptr;
		return R;
	}

	void Close();

private:
	FForgeFFISessionPtr Raw = nullptr;
};

/**
 * Static facade for the loaded DLL. Not a singleton per se — the
 * resolved function pointers are module-level globals the wrapper
 * functions close over. Instance methods are not meaningful here.
 */
class FORGESOURCECONTROL_API FForgeFFI
{
public:
	/** Load `forge_ffi.dll` + resolve all exported symbols. */
	static void Initialize();

	/** Close the DLL handle if one was opened. Idempotent. */
	static void Shutdown();

	/** `true` after a successful [`Initialize`]; `false` when the
	 *  library wasn't found or one of the required symbols is missing. */
	static bool IsAvailable();

	/**
	 * Returns the FFI ABI version the loaded library advertises, or
	 * `-1` when the library isn't loaded. The plugin pins a minimum
	 * so an old `.dll` can't silently speak to a newer header layout.
	 */
	static int32 GetAbiVersion();

	/** Library version string (the Rust crate's CARGO_PKG_VERSION). */
	static FString GetLibraryVersion();

	// ── Session lifecycle ────────────────────────────────────────

	/**
	 * Open a session rooted at the given local workspace path. On
	 * failure returns an invalid session and populates `OutError`.
	 * `WorkspacePath` must already live on disk — the plugin's
	 * `FForgeSourceControlProvider::Init` has the canonical resolution.
	 */
	static FForgeFFISession OpenSession(const FString& WorkspacePath, FText& OutError);

	// ── Typed wrappers (Phase 4c.2 migrates individual workers) ──

	/**
	 * Fetch a JSON string describing the current workspace status.
	 * Blocking — call from the worker thread, never the game thread.
	 * Returns empty string on failure (check `OutError`).
	 */
	static FString StatusJson(const FForgeFFISession& Session, FText& OutError);

	/**
	 * List active locks on the workspace's default remote as a JSON
	 * array. Blocking.
	 */
	static FString LockListJson(const FForgeFFISession& Session, FText& OutError);

	/**
	 * Acquire a lock on `Path` with an optional `Reason`. Returns
	 * `true` on success. `OutError` carries a structured message on
	 * `CONFLICT` so the caller can render a "held by X" toast.
	 */
	static bool LockAcquire(
		const FForgeFFISession& Session,
		const FString& Path,
		const FString& Reason,
		FText& OutError);

	/** Release the caller's own lock on `Path`. */
	static bool LockRelease(
		const FForgeFFISession& Session,
		const FString& Path,
		FText& OutError);

	/**
	 * One-shot introspection call that replaces several plugin-side
	 * subprocess invocations: returns workspace_root, workspace_id,
	 * repo name, default remote URL, current HEAD, user identity, and
	 * workflow mode as a single JSON document. See the Rust doc on
	 * `forge_workspace_info_json` for the full schema.
	 */
	static FString WorkspaceInfoJson(const FForgeFFISession& Session, FText& OutError);

	/**
	 * Returns the current branch name, or an empty FString when HEAD
	 * is detached. `OutError` stays empty in the detached case —
	 * detached HEAD is not an error condition.
	 */
	static FString CurrentBranch(const FForgeFFISession& Session, FText& OutError);

private:
	// Non-instantiable.
	FForgeFFI() = delete;
};
