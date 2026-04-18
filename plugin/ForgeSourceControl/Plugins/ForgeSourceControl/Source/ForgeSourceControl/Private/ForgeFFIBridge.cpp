// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#include "ForgeFFIBridge.h"

#include "HAL/PlatformProcess.h"
#include "Misc/Paths.h"
#include "Interfaces/IPluginManager.h"
#include "ISourceControlModule.h"

#define LOCTEXT_NAMESPACE "ForgeSourceControl"

namespace
{
	// Resolved DLL + function pointers. All globals because the facade
	// is callable from any worker thread after Initialize; no
	// synchronisation is needed because Initialize is called from
	// `StartupModule` (main thread, before any worker runs) and
	// Shutdown from `ShutdownModule` (main thread, after everything).
	void* GDllHandle = nullptr;

#if FORGE_FFI_HAVE_HEADER
	// Typed function-pointer mirrors for each C ABI entry we use. Kept
	// close to the header so a signature mismatch between the header
	// and the `.dll` surfaces at module load, not on first call.
	typedef const char*           (*PFN_forge_version)();
	typedef int                   (*PFN_forge_abi_version)();
	typedef forge_session_t*      (*PFN_forge_session_open)(const char*, forge_error_t*);
	typedef void                  (*PFN_forge_session_close)(forge_session_t*);
	typedef void                  (*PFN_forge_error_free)(forge_error_t*);
	typedef void                  (*PFN_forge_string_free)(char*);
	typedef char*                 (*PFN_forge_status_json)(forge_session_t*, forge_error_t*);
	typedef char*                 (*PFN_forge_lock_list_json)(forge_session_t*, forge_error_t*);
	typedef int                   (*PFN_forge_lock_acquire)(forge_session_t*, const char*, const char*, forge_error_t*);
	typedef int                   (*PFN_forge_lock_release)(forge_session_t*, const char*, forge_error_t*);
	typedef char*                 (*PFN_forge_workspace_info_json)(forge_session_t*, forge_error_t*);
	typedef char*                 (*PFN_forge_current_branch)(forge_session_t*, forge_error_t*);
	typedef int                   (*PFN_forge_add_paths)(forge_session_t*, const char*, forge_error_t*);
	typedef int                   (*PFN_forge_commit)(forge_session_t*, const char*, forge_error_t*);
	typedef int                   (*PFN_forge_push)(forge_session_t*, int, forge_error_t*);
	typedef int                   (*PFN_forge_pull)(forge_session_t*, forge_error_t*);
	typedef int                   (*PFN_forge_subscribe_lock_events)(forge_session_t*, forge_error_t*);
	typedef char*                 (*PFN_forge_poll_lock_events_json)(forge_session_t*, forge_error_t*);

	PFN_forge_version         GForgeVersion         = nullptr;
	PFN_forge_abi_version     GForgeAbiVersion      = nullptr;
	PFN_forge_session_open    GForgeSessionOpen     = nullptr;
	PFN_forge_session_close   GForgeSessionClose    = nullptr;
	PFN_forge_error_free      GForgeErrorFree       = nullptr;
	PFN_forge_string_free     GForgeStringFree      = nullptr;
	PFN_forge_status_json     GForgeStatusJson      = nullptr;
	PFN_forge_lock_list_json      GForgeLockListJson      = nullptr;
	PFN_forge_lock_acquire        GForgeLockAcquire       = nullptr;
	PFN_forge_lock_release        GForgeLockRelease       = nullptr;
	PFN_forge_workspace_info_json GForgeWorkspaceInfoJson = nullptr;
	PFN_forge_current_branch      GForgeCurrentBranch     = nullptr;
	PFN_forge_add_paths           GForgeAddPaths          = nullptr;
	PFN_forge_commit              GForgeCommit            = nullptr;
	PFN_forge_push                GForgePush              = nullptr;
	PFN_forge_pull                GForgePull              = nullptr;
	PFN_forge_subscribe_lock_events GForgeSubscribeLockEvents = nullptr;
	PFN_forge_poll_lock_events_json GForgePollLockEventsJson  = nullptr;

	// Minimum ABI version the plugin accepts. Bump when the header
	// adds or breaks exported signatures so a stale `.dll` on disk
	// after a plugin upgrade can't silently load and crash on a
	// later call.
	constexpr int32 kMinSupportedAbi = 4;

	/** Resolve the expected location of `forge_ffi.dll` next to the
	 *  plugin's Binaries/ dir so the user doesn't need to PATH-install. */
	FString ResolveDllPath()
	{
		TSharedPtr<IPlugin> Plugin = IPluginManager::Get().FindPlugin(TEXT("ForgeSourceControl"));
		if (!Plugin.IsValid())
		{
			return FString();
		}
		const FString Base = Plugin->GetBaseDir();
	#if PLATFORM_WINDOWS
		return FPaths::Combine(Base, TEXT("Binaries"), TEXT("Win64"), TEXT("forge_ffi.dll"));
	#elif PLATFORM_MAC
		return FPaths::Combine(Base, TEXT("Binaries"), TEXT("Mac"), TEXT("libforge_ffi.dylib"));
	#elif PLATFORM_LINUX
		return FPaths::Combine(Base, TEXT("Binaries"), TEXT("Linux"), TEXT("libforge_ffi.so"));
	#else
		return FString();
	#endif
	}

	/** Helper: populate FText from a forge_error_t populated by the
	 *  library, then free the message. Safe on a zeroed struct. */
	FText ConsumeError(forge_error_t& Err, forge_status_t FallbackCode, const TCHAR* FallbackMsg)
	{
		FText Out;
		if (Err.message != nullptr)
		{
			const FString Msg(UTF8_TO_TCHAR(Err.message));
			Out = FText::FromString(FString::Printf(TEXT("forge-ffi: %s"), *Msg));
		}
		else
		{
			Out = FText::FromString(FString::Printf(
				TEXT("forge-ffi (status %d): %s"),
				(int32)FallbackCode,
				FallbackMsg));
		}
		if (GForgeErrorFree != nullptr)
		{
			GForgeErrorFree(&Err);
		}
		return Out;
	}

	/** Turn a `char*` the library allocated into an FString + free it. */
	FString ConsumeOwnedString(char* Owned)
	{
		FString Out;
		if (Owned != nullptr)
		{
			Out = FString(UTF8_TO_TCHAR(Owned));
			if (GForgeStringFree != nullptr)
			{
				GForgeStringFree(Owned);
			}
		}
		return Out;
	}
#endif // FORGE_FFI_HAVE_HEADER
}

// ── FForgeFFISession ────────────────────────────────────────────────────────

void FForgeFFISession::Close()
{
#if FORGE_FFI_HAVE_HEADER
	if (Raw != nullptr && GForgeSessionClose != nullptr)
	{
		GForgeSessionClose(Raw);
	}
#endif
	Raw = nullptr;
}

// ── FForgeFFI facade ────────────────────────────────────────────────────────

void FForgeFFI::Initialize()
{
#if FORGE_FFI_HAVE_HEADER
	if (GDllHandle != nullptr)
	{
		return; // Idempotent.
	}

	const FString DllPath = ResolveDllPath();
	if (DllPath.IsEmpty() || !FPaths::FileExists(DllPath))
	{
		UE_LOG(LogSourceControl, Warning,
			TEXT("ForgeFFI: library not found at %s — falling back to CLI subprocess path."),
			*DllPath);
		return;
	}

	GDllHandle = FPlatformProcess::GetDllHandle(*DllPath);
	if (GDllHandle == nullptr)
	{
		UE_LOG(LogSourceControl, Warning,
			TEXT("ForgeFFI: GetDllHandle failed for %s — falling back to CLI subprocess path."),
			*DllPath);
		return;
	}

	#define FFI_RESOLVE(Name) \
		G##Name = reinterpret_cast<PFN_##Name>(FPlatformProcess::GetDllExport(GDllHandle, TEXT(#Name))); \
		if (G##Name == nullptr) \
		{ \
			UE_LOG(LogSourceControl, Warning, \
				TEXT("ForgeFFI: missing symbol %s in %s — falling back to CLI."), \
				TEXT(#Name), *DllPath); \
			Shutdown(); \
			return; \
		}

	// Keep the names aligned with the typedef block above.
	FFI_RESOLVE(forge_version);
	FFI_RESOLVE(forge_abi_version);
	FFI_RESOLVE(forge_session_open);
	FFI_RESOLVE(forge_session_close);
	FFI_RESOLVE(forge_error_free);
	FFI_RESOLVE(forge_string_free);
	FFI_RESOLVE(forge_status_json);
	FFI_RESOLVE(forge_lock_list_json);
	FFI_RESOLVE(forge_lock_acquire);
	FFI_RESOLVE(forge_lock_release);
	FFI_RESOLVE(forge_workspace_info_json);
	FFI_RESOLVE(forge_current_branch);
	FFI_RESOLVE(forge_add_paths);
	FFI_RESOLVE(forge_commit);
	FFI_RESOLVE(forge_push);
	FFI_RESOLVE(forge_pull);
	FFI_RESOLVE(forge_subscribe_lock_events);
	FFI_RESOLVE(forge_poll_lock_events_json);

	#undef FFI_RESOLVE

	const int32 Abi = GForgeAbiVersion != nullptr ? GForgeAbiVersion() : -1;
	if (Abi < kMinSupportedAbi)
	{
		UE_LOG(LogSourceControl, Warning,
			TEXT("ForgeFFI: library ABI %d is older than the plugin's minimum %d — unloading."),
			Abi, kMinSupportedAbi);
		Shutdown();
		return;
	}

	const FString Ver = GForgeVersion != nullptr ? FString(UTF8_TO_TCHAR(GForgeVersion())) : TEXT("?");
	UE_LOG(LogSourceControl, Log,
		TEXT("ForgeFFI: loaded %s (abi %d) from %s"),
		*Ver, Abi, *DllPath);
#else
	UE_LOG(LogSourceControl, Warning,
		TEXT("ForgeFFI: forge_ffi.h not available at plugin compile time — the bridge is a no-op."));
#endif
}

void FForgeFFI::Shutdown()
{
#if FORGE_FFI_HAVE_HEADER
	if (GDllHandle != nullptr)
	{
		FPlatformProcess::FreeDllHandle(GDllHandle);
		GDllHandle = nullptr;
	}
	GForgeVersion = nullptr;
	GForgeAbiVersion = nullptr;
	GForgeSessionOpen = nullptr;
	GForgeSessionClose = nullptr;
	GForgeErrorFree = nullptr;
	GForgeStringFree = nullptr;
	GForgeStatusJson = nullptr;
	GForgeLockListJson = nullptr;
	GForgeLockAcquire = nullptr;
	GForgeLockRelease = nullptr;
	GForgeWorkspaceInfoJson = nullptr;
	GForgeCurrentBranch = nullptr;
	GForgeAddPaths = nullptr;
	GForgeCommit = nullptr;
	GForgePush = nullptr;
	GForgePull = nullptr;
	GForgeSubscribeLockEvents = nullptr;
	GForgePollLockEventsJson = nullptr;
#endif
}

bool FForgeFFI::IsAvailable()
{
#if FORGE_FFI_HAVE_HEADER
	return GDllHandle != nullptr && GForgeSessionOpen != nullptr;
#else
	return false;
#endif
}

int32 FForgeFFI::GetAbiVersion()
{
#if FORGE_FFI_HAVE_HEADER
	return GForgeAbiVersion != nullptr ? (int32)GForgeAbiVersion() : -1;
#else
	return -1;
#endif
}

FString FForgeFFI::GetLibraryVersion()
{
#if FORGE_FFI_HAVE_HEADER
	return GForgeVersion != nullptr
		? FString(UTF8_TO_TCHAR(GForgeVersion()))
		: FString();
#else
	return FString();
#endif
}

FForgeFFISession FForgeFFI::OpenSession(const FString& WorkspacePath, FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable())
	{
		OutError = LOCTEXT("FFIUnavailable",
			"The Forge FFI library is not loaded. Check the editor log for details.");
		return FForgeFFISession();
	}

	const FTCHARToUTF8 PathUtf8(*WorkspacePath);
	forge_error_t Err = {};
	forge_session_t* Raw = GForgeSessionOpen(PathUtf8.Get(), &Err);
	if (Raw == nullptr)
	{
		OutError = ConsumeError(Err, (forge_status_t)2, TEXT("forge_session_open failed"));
		return FForgeFFISession();
	}
	OutError = FText::GetEmpty();
	return FForgeFFISession(Raw);
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return FForgeFFISession();
#endif
}

FString FForgeFFI::StatusJson(const FForgeFFISession& Session, FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("StatusUnavailable", "FFI session is not available.");
		return FString();
	}
	forge_error_t Err = {};
	char* Raw = GForgeStatusJson(Session.Get(), &Err);
	if (Raw == nullptr)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_status_json failed"));
		return FString();
	}
	OutError = FText::GetEmpty();
	return ConsumeOwnedString(Raw);
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return FString();
#endif
}

FString FForgeFFI::LockListJson(const FForgeFFISession& Session, FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("LockListUnavailable", "FFI session is not available.");
		return FString();
	}
	forge_error_t Err = {};
	char* Raw = GForgeLockListJson(Session.Get(), &Err);
	if (Raw == nullptr)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_lock_list_json failed"));
		return FString();
	}
	OutError = FText::GetEmpty();
	return ConsumeOwnedString(Raw);
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return FString();
#endif
}

bool FForgeFFI::LockAcquire(
	const FForgeFFISession& Session,
	const FString& Path,
	const FString& Reason,
	FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("LockAcquireUnavailable", "FFI session is not available.");
		return false;
	}
	const FTCHARToUTF8 PathUtf8(*Path);
	const FTCHARToUTF8 ReasonUtf8(*Reason);
	forge_error_t Err = {};
	const int Rc = GForgeLockAcquire(Session.Get(), PathUtf8.Get(), ReasonUtf8.Get(), &Err);
	if (Rc != 0)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_lock_acquire failed"));
		return false;
	}
	OutError = FText::GetEmpty();
	return true;
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return false;
#endif
}

bool FForgeFFI::LockRelease(
	const FForgeFFISession& Session,
	const FString& Path,
	FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("LockReleaseUnavailable", "FFI session is not available.");
		return false;
	}
	const FTCHARToUTF8 PathUtf8(*Path);
	forge_error_t Err = {};
	const int Rc = GForgeLockRelease(Session.Get(), PathUtf8.Get(), &Err);
	if (Rc != 0)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_lock_release failed"));
		return false;
	}
	OutError = FText::GetEmpty();
	return true;
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return false;
#endif
}

FString FForgeFFI::WorkspaceInfoJson(const FForgeFFISession& Session, FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("InfoUnavailable", "FFI session is not available.");
		return FString();
	}
	forge_error_t Err = {};
	char* Raw = GForgeWorkspaceInfoJson(Session.Get(), &Err);
	if (Raw == nullptr)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_workspace_info_json failed"));
		return FString();
	}
	OutError = FText::GetEmpty();
	return ConsumeOwnedString(Raw);
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return FString();
#endif
}

bool FForgeFFI::AddPaths(
	const FForgeFFISession& Session,
	const TArray<FString>& Paths,
	FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("AddUnavailable", "FFI session is not available.");
		return false;
	}
	// Serialise the path list to JSON. Done with a tiny hand-rolled
	// builder — pulling in Json module here would cost a dependency
	// for one call. JSON-escaping covers the two characters that
	// actually matter (`"` and `\`); anything fancier lives in
	// FJsonSerializer on the consumers that need it.
	FString Json = TEXT("[");
	for (int32 Idx = 0; Idx < Paths.Num(); ++Idx)
	{
		if (Idx > 0) { Json += TEXT(","); }
		FString Escaped = Paths[Idx];
		Escaped.ReplaceInline(TEXT("\\"), TEXT("\\\\"));
		Escaped.ReplaceInline(TEXT("\""), TEXT("\\\""));
		Json += TEXT("\"");
		Json += Escaped;
		Json += TEXT("\"");
	}
	Json += TEXT("]");

	const FTCHARToUTF8 JsonUtf8(*Json);
	forge_error_t Err = {};
	const int Rc = GForgeAddPaths(Session.Get(), JsonUtf8.Get(), &Err);
	if (Rc != 0)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_add_paths failed"));
		return false;
	}
	OutError = FText::GetEmpty();
	return true;
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return false;
#endif
}

bool FForgeFFI::Commit(
	const FForgeFFISession& Session,
	const FString& Message,
	FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("CommitUnavailable", "FFI session is not available.");
		return false;
	}
	const FTCHARToUTF8 MsgUtf8(*Message);
	forge_error_t Err = {};
	const int Rc = GForgeCommit(Session.Get(), MsgUtf8.Get(), &Err);
	if (Rc != 0)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_commit failed"));
		return false;
	}
	OutError = FText::GetEmpty();
	return true;
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return false;
#endif
}

bool FForgeFFI::Push(
	const FForgeFFISession& Session,
	bool bForce,
	FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("PushUnavailable", "FFI session is not available.");
		return false;
	}
	forge_error_t Err = {};
	const int Rc = GForgePush(Session.Get(), bForce ? 1 : 0, &Err);
	if (Rc != 0)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_push failed"));
		return false;
	}
	OutError = FText::GetEmpty();
	return true;
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return false;
#endif
}

bool FForgeFFI::SubscribeLockEvents(const FForgeFFISession& Session, FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("SubUnavailable", "FFI session is not available.");
		return false;
	}
	forge_error_t Err = {};
	const int Rc = GForgeSubscribeLockEvents(Session.Get(), &Err);
	if (Rc != 0)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_subscribe_lock_events failed"));
		return false;
	}
	OutError = FText::GetEmpty();
	return true;
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return false;
#endif
}

FString FForgeFFI::PollLockEventsJson(const FForgeFFISession& Session, FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("PollUnavailable", "FFI session is not available.");
		return FString();
	}
	forge_error_t Err = {};
	char* Raw = GForgePollLockEventsJson(Session.Get(), &Err);
	if (Raw == nullptr)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_poll_lock_events_json failed"));
		return FString();
	}
	OutError = FText::GetEmpty();
	return ConsumeOwnedString(Raw);
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return FString();
#endif
}

bool FForgeFFI::Pull(
	const FForgeFFISession& Session,
	FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("PullUnavailable", "FFI session is not available.");
		return false;
	}
	forge_error_t Err = {};
	const int Rc = GForgePull(Session.Get(), &Err);
	if (Rc != 0)
	{
		OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_pull failed"));
		return false;
	}
	OutError = FText::GetEmpty();
	return true;
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return false;
#endif
}

FString FForgeFFI::CurrentBranch(const FForgeFFISession& Session, FText& OutError)
{
#if FORGE_FFI_HAVE_HEADER
	if (!IsAvailable() || !Session.IsValid())
	{
		OutError = LOCTEXT("BranchUnavailable", "FFI session is not available.");
		return FString();
	}
	forge_error_t Err = {};
	char* Raw = GForgeCurrentBranch(Session.Get(), &Err);
	if (Raw == nullptr)
	{
		// Null + OK = detached HEAD; null + non-OK = real error.
		if (Err.code != 0)
		{
			OutError = ConsumeError(Err, (forge_status_t)1, TEXT("forge_current_branch failed"));
		}
		else
		{
			OutError = FText::GetEmpty();
		}
		return FString();
	}
	OutError = FText::GetEmpty();
	return ConsumeOwnedString(Raw);
#else
	OutError = LOCTEXT("FFIHeaderMissing", "forge_ffi.h was not available at plugin compile time.");
	return FString();
#endif
}

#undef LOCTEXT_NAMESPACE
