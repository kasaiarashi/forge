// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#include "ForgeSourceControlWorkers.h"
#include "ForgeSourceControlCommand.h"
#include "ForgeSourceControlProvider.h"
#include "ISourceControlModule.h"
#include "SourceControlOperations.h"
#include "Misc/Paths.h"
#include "Serialization/JsonReader.h"
#include "Serialization/JsonSerializer.h"

#define LOCTEXT_NAMESPACE "ForgeSourceControl"

// ── Connect ─────────────────────────────────────────────────────────────────

bool FForgeConnectWorker::Execute(FForgeSourceControlCommand& InCommand)
{
	ProviderRef = &InCommand.Provider;
	const bool bAvailable = !InCommand.Provider.GetWorkspaceRoot().IsEmpty();
	if (!bAvailable)
	{
		InCommand.ErrorMessages.Add(TEXT("No Forge workspace found"));
	}
	InCommand.MarkOperationCompleted(bAvailable);
	return bAvailable;
}

bool FForgeConnectWorker::UpdateStates()
{
	return false;
}

// ── UpdateStatus ────────────────────────────────────────────────────────────

bool FForgeUpdateStatusWorker::Execute(FForgeSourceControlCommand& InCommand)
{
	ProviderRef = &InCommand.Provider;
	FForgeSourceControlProvider& Provider = InCommand.Provider;
	WorkspaceRoot = Provider.GetWorkspaceRoot();

	TSharedPtr<FJsonObject> Result;
	if (!Provider.RunForgeCommand(TEXT("status"), Result) || !Result.IsValid())
	{
		InCommand.ErrorMessages.Add(TEXT("Failed to run forge status"));
		InCommand.MarkOperationCompleted(false);
		return false;
	}

	States.Empty();
	LockOwners.Empty();

	auto ParseArray = [&](const FString& FieldName, FForgeSourceControlState::EFileState State)
	{
		const TArray<TSharedPtr<FJsonValue>>* Files;
		if (Result->TryGetArrayField(FieldName, Files))
		{
			for (const auto& Val : *Files)
			{
				FString Path = FPaths::ConvertRelativePathToFull(WorkspaceRoot, Val->AsString());
				FPaths::NormalizeFilename(Path);
				FPaths::CollapseRelativeDirectories(Path);
				States.Add(Path, State);
			}
		}
	};

	ParseArray(TEXT("staged_new"), FForgeSourceControlState::EFileState::Added);
	ParseArray(TEXT("staged_modified"), FForgeSourceControlState::EFileState::Modified);
	ParseArray(TEXT("staged_deleted"), FForgeSourceControlState::EFileState::Deleted);
	ParseArray(TEXT("modified"), FForgeSourceControlState::EFileState::Modified);
	ParseArray(TEXT("deleted"), FForgeSourceControlState::EFileState::Deleted);
	ParseArray(TEXT("untracked"), FForgeSourceControlState::EFileState::Untracked);

	const TArray<TSharedPtr<FJsonValue>>* LockedFiles;
	if (Result->TryGetArrayField(TEXT("locked"), LockedFiles))
	{
		for (const auto& Val : *LockedFiles)
		{
			const TSharedPtr<FJsonObject>* LockObj;
			if (Val->TryGetObject(LockObj))
			{
				FString LockPath, LockOwner;
				(*LockObj)->TryGetStringField(TEXT("path"), LockPath);
				(*LockObj)->TryGetStringField(TEXT("owner"), LockOwner);
				FString FullPath = FPaths::ConvertRelativePathToFull(WorkspaceRoot, LockPath);
				FPaths::NormalizeFilename(FullPath);
				FPaths::CollapseRelativeDirectories(FullPath);

				const FString& CurrentUser = Provider.GetCurrentUserName();
				if (LockOwner == CurrentUser)
				{
					States.Add(FullPath, FForgeSourceControlState::EFileState::Locked);
				}
				else
				{
					States.Add(FullPath, FForgeSourceControlState::EFileState::LockedByOther);
				}
				LockOwners.Add(FullPath, LockOwner);
			}
		}
	}

	InCommand.MarkOperationCompleted(true);
	return true;
}

bool FForgeUpdateStatusWorker::UpdateStates()
{
	return ProviderRef->UpdateCachedStates(States, LockOwners);
}

// ── CheckOut (Lock) ─────────────────────────────────────────────────────────

bool FForgeCheckOutWorker::Execute(FForgeSourceControlCommand& InCommand)
{
	ProviderRef = &InCommand.Provider;
	FForgeSourceControlProvider& Provider = InCommand.Provider;
	const FString WsRoot = Provider.GetWorkspaceRoot();
	bool bSuccess = true;

	// Phase 4c.2 — prefer FFI when the library is loaded. Each lock
	// used to cost one CreateProcess + forge CLI startup (~15 ms
	// cold). Through the bridge it's a direct gRPC call on the
	// session's owned tokio runtime, so N files = N round-trips, no
	// per-file process overhead. A missing library or failed session
	// transparently falls back to the legacy subprocess path.
	const FForgeFFISession* FFI = Provider.GetFFISession();

	for (const FString& File : InCommand.Files)
	{
		FString RelPath = File;
		FPaths::MakePathRelativeTo(RelPath, *(WsRoot / TEXT("")));

		if (FFI != nullptr)
		{
			FText LockError;
			if (FForgeFFI::LockAcquire(*FFI, RelPath, FString(), LockError))
			{
				LockedFiles.Add(File);
				continue;
			}
			InCommand.ErrorMessages.Add(FString::Printf(
				TEXT("Lock denied for '%s': %s"), *RelPath, *LockError.ToString()));
			bSuccess = false;
			break;
		}

		// CLI fallback. Kept in place for:
		//  - dev builds where forge_ffi.dll isn't next to the editor.
		//  - transient session-open failures logged by GetFFISession.
		TSharedPtr<FJsonObject> JsonResult;
		if (Provider.RunForgeCommand(FString::Printf(TEXT("lock \"%s\""), *RelPath), JsonResult))
		{
			bool bOk = false;
			if (JsonResult.IsValid() && JsonResult->TryGetBoolField(TEXT("ok"), bOk) && bOk)
			{
				LockedFiles.Add(File);
			}
			else
			{
				FString Error;
				if (JsonResult.IsValid()) { JsonResult->TryGetStringField(TEXT("error"), Error); }
				InCommand.ErrorMessages.Add(FString::Printf(TEXT("Lock denied for '%s': %s"), *RelPath, *Error));
				bSuccess = false;
				break;
			}
		}
		else
		{
			InCommand.ErrorMessages.Add(FString::Printf(TEXT("Failed to lock '%s'"), *RelPath));
			bSuccess = false;
			break;
		}
	}

	InCommand.MarkOperationCompleted(bSuccess);
	return bSuccess;
}

bool FForgeCheckOutWorker::UpdateStates()
{
	for (const FString& File : LockedFiles)
	{
		TSharedRef<FForgeSourceControlState> State = ProviderRef->GetStateInternal(File);
		State->FileState = FForgeSourceControlState::EFileState::Locked;
		State->LockOwner = ProviderRef->GetCurrentUserName();
	}
	return LockedFiles.Num() > 0;
}

// ── CheckIn (Commit + Push) ─────────────────────────────────────────────────

bool FForgeCheckInWorker::Execute(FForgeSourceControlCommand& InCommand)
{
	ProviderRef = &InCommand.Provider;
	FForgeSourceControlProvider& Provider = InCommand.Provider;
	const FString WsRoot = Provider.GetWorkspaceRoot();

	TSharedRef<FCheckIn, ESPMode::ThreadSafe> CheckInOp = StaticCastSharedRef<FCheckIn>(InCommand.Operation);
	FString Message = CheckInOp->GetDescription().ToString();
	if (Message.IsEmpty()) { Message = TEXT("Checked in from Unreal Editor"); }
	Message.ReplaceInline(TEXT("\""), TEXT("\\\""));

	// Stage files.
	for (const FString& File : InCommand.Files)
	{
		FString RelPath = File;
		FPaths::MakePathRelativeTo(RelPath, *(WsRoot / TEXT("")));
		UE_LOG(LogSourceControl, Log, TEXT("Forge CheckIn: staging '%s' (rel: '%s')"), *File, *RelPath);
		if (!Provider.RunForgeCommandRaw(FString::Printf(TEXT("add \"%s\""), *RelPath)))
		{
			InCommand.ErrorMessages.Add(FString::Printf(TEXT("Failed to stage '%s'"), *RelPath));
			InCommand.MarkOperationCompleted(false);
			return false;
		}
	}

	// Commit.
	if (!Provider.RunForgeCommandRaw(FString::Printf(TEXT("commit -m \"%s\""), *Message)))
	{
		InCommand.ErrorMessages.Add(TEXT("Failed to create commit"));
		InCommand.MarkOperationCompleted(false);
		return false;
	}

	// Unlock files (non-fatal). Prefer FFI when available — one
	// gRPC round-trip per file on the session's owned runtime,
	// instead of per-file subprocess spawn. Either path is
	// best-effort: unlock failure doesn't block the check-in.
	{
		const FForgeFFISession* FFI = Provider.GetFFISession();
		for (const FString& File : InCommand.Files)
		{
			FString RelPath = File;
			FPaths::MakePathRelativeTo(RelPath, *(WsRoot / TEXT("")));
			if (FFI != nullptr)
			{
				FText IgnoredErr;
				FForgeFFI::LockRelease(*FFI, RelPath, IgnoredErr);
				// We deliberately drop IgnoredErr — unlock failures
				// are non-fatal and already logged by the Rust side.
			}
			else
			{
				TSharedPtr<FJsonObject> Unused;
				Provider.RunForgeCommand(FString::Printf(TEXT("unlock \"%s\""), *RelPath), Unused);
			}
		}
	}

	// Push.
	if (!Provider.RunForgeCommandRaw(TEXT("push")))
	{
		InCommand.ErrorMessages.Add(TEXT("Commit created but push failed"));
		InCommand.MarkOperationCompleted(false);
		return false;
	}

	InCommand.InfoMessages.Add(TEXT("Successfully checked in and pushed"));
	InCommand.MarkOperationCompleted(true);
	return true;
}

bool FForgeCheckInWorker::UpdateStates()
{
	// Trigger a deferred status refresh so the UI reflects the post-commit state
	// (committed files become Unmodified, locks released). The provider will
	// queue an async UpdateStatus and broadcast when it completes.
	if (ProviderRef)
	{
		ProviderRef->RefreshStatusAsync();
	}
	return false;
}

// ── MarkForAdd ──────────────────────────────────────────────────────────────

bool FForgeMarkForAddWorker::Execute(FForgeSourceControlCommand& InCommand)
{
	ProviderRef = &InCommand.Provider;
	FForgeSourceControlProvider& Provider = InCommand.Provider;
	const FString WsRoot = Provider.GetWorkspaceRoot();

	for (const FString& File : InCommand.Files)
	{
		FString RelPath = File;
		FPaths::MakePathRelativeTo(RelPath, *(WsRoot / TEXT("")));
		if (!Provider.RunForgeCommandRaw(FString::Printf(TEXT("add \"%s\""), *RelPath)))
		{
			InCommand.ErrorMessages.Add(FString::Printf(TEXT("Failed to add '%s'"), *RelPath));
			InCommand.MarkOperationCompleted(false);
			return false;
		}
	}

	InCommand.MarkOperationCompleted(true);
	return true;
}

bool FForgeMarkForAddWorker::UpdateStates()
{
	return false;
}

// ── Revert ──────────────────────────────────────────────────────────────────

bool FForgeRevertWorker::Execute(FForgeSourceControlCommand& InCommand)
{
	ProviderRef = &InCommand.Provider;
	FForgeSourceControlProvider& Provider = InCommand.Provider;
	const FString WsRoot = Provider.GetWorkspaceRoot();

	// See CheckIn unlock comment — FFI-first, subprocess fallback.
	const FForgeFFISession* FFI = Provider.GetFFISession();
	for (const FString& File : InCommand.Files)
	{
		FString RelPath = File;
		FPaths::MakePathRelativeTo(RelPath, *(WsRoot / TEXT("")));

		if (!Provider.RunForgeCommandRaw(FString::Printf(TEXT("restore \"%s\""), *RelPath)))
		{
			InCommand.ErrorMessages.Add(FString::Printf(TEXT("Failed to restore '%s'"), *RelPath));
			InCommand.MarkOperationCompleted(false);
			return false;
		}

		// Unlock (non-fatal).
		if (FFI != nullptr)
		{
			FText IgnoredErr;
			FForgeFFI::LockRelease(*FFI, RelPath, IgnoredErr);
		}
		else
		{
			TSharedPtr<FJsonObject> Unused;
			Provider.RunForgeCommand(FString::Printf(TEXT("unlock \"%s\""), *RelPath), Unused);
		}
	}

	InCommand.MarkOperationCompleted(true);
	return true;
}

bool FForgeRevertWorker::UpdateStates()
{
	return false;
}

// ── Delete ──────────────────────────────────────────────────────────────────

bool FForgeDeleteWorker::Execute(FForgeSourceControlCommand& InCommand)
{
	ProviderRef = &InCommand.Provider;
	FForgeSourceControlProvider& Provider = InCommand.Provider;
	const FString WsRoot = Provider.GetWorkspaceRoot();

	for (const FString& File : InCommand.Files)
	{
		FString RelPath = File;
		FPaths::MakePathRelativeTo(RelPath, *(WsRoot / TEXT("")));
		if (!Provider.RunForgeCommandRaw(FString::Printf(TEXT("rm \"%s\""), *RelPath)))
		{
			InCommand.ErrorMessages.Add(FString::Printf(TEXT("Failed to delete '%s'"), *RelPath));
			InCommand.MarkOperationCompleted(false);
			return false;
		}
	}

	InCommand.MarkOperationCompleted(true);
	return true;
}

bool FForgeDeleteWorker::UpdateStates()
{
	return false;
}

// ── Sync (Pull) ─────────────────────────────────────────────────────────────

bool FForgeSyncWorker::Execute(FForgeSourceControlCommand& InCommand)
{
	ProviderRef = &InCommand.Provider;
	// No-op: UE's AssetViewUtils::SyncPathsFromSourceControl unconditionally calls
	// UPackageTools::ReloadPackages on every loaded package after Sync returns,
	// which crashes the render thread when engine packages are reloaded.
	// Users can run `forge pull` manually until we wire up a deferred reload path.
	InCommand.MarkOperationCompleted(true);
	return true;
}

bool FForgeSyncWorker::UpdateStates()
{
	return false;
}

#undef LOCTEXT_NAMESPACE
