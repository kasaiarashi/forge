// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#include "ForgeSourceControlProvider.h"
#include "ForgeSourceControlState.h"
#include "ISourceControlModule.h"
#include "Widgets/SNullWidget.h"
#include "Misc/Paths.h"
#include "Misc/FileHelper.h"
#include "Serialization/JsonReader.h"
#include "Serialization/JsonSerializer.h"
#include "HAL/PlatformProcess.h"

#define LOCTEXT_NAMESPACE "ForgeSourceControl"

void FForgeSourceControlProvider::Init(bool bForceConnection)
{
	// Locate the forge executable.
	ForgeExePath = TEXT("forge");

	// Fast check: just see if .forge directory exists in the project.
	const FString ForgeDir = FPaths::ConvertRelativePathToFull(FPaths::ProjectDir()) / TEXT(".forge");
	bIsAvailable = FPaths::DirectoryExists(ForgeDir);

	if (bIsAvailable)
	{
		UE_LOG(LogSourceControl, Log, TEXT("Forge: workspace found at %s"), *ForgeDir);
	}
}

void FForgeSourceControlProvider::Close()
{
	bIsAvailable = false;
	StateCache.Empty();
}

const FName& FForgeSourceControlProvider::GetName() const
{
	static const FName ProviderName("Forge");
	return ProviderName;
}

FText FForgeSourceControlProvider::GetStatusText() const
{
	if (bIsAvailable)
	{
		return LOCTEXT("StatusAvailable", "Connected to Forge workspace");
	}
	return LOCTEXT("StatusUnavailable", "No Forge workspace found");
}

TMap<ISourceControlProvider::EStatus, FString> FForgeSourceControlProvider::GetStatus() const
{
	TMap<EStatus, FString> Result;
	Result.Add(EStatus::Enabled, IsEnabled() ? TEXT("Yes") : TEXT("No"));
	Result.Add(EStatus::Connected, IsAvailable() ? TEXT("Yes") : TEXT("No"));
	return Result;
}

bool FForgeSourceControlProvider::IsEnabled() const
{
	return true;
}

bool FForgeSourceControlProvider::IsAvailable() const
{
	return bIsAvailable;
}

void FForgeSourceControlProvider::RefreshStatusCache()
{
	TSharedPtr<FJsonObject> Result;
	if (RunForgeCommand(TEXT("status --json"), Result) && Result.IsValid())
	{
		StateCache.Empty();
		const FString ProjDir = FPaths::ConvertRelativePathToFull(FPaths::ProjectDir());

		auto ParseArray = [&](const FString& FieldName, FForgeSourceControlState::EFileState State)
		{
			const TArray<TSharedPtr<FJsonValue>>* Files;
			if (Result->TryGetArrayField(FieldName, Files))
			{
				for (const auto& Val : *Files)
				{
					FString Path = FPaths::ConvertRelativePathToFull(ProjDir, Val->AsString());
					FPaths::NormalizeFilename(Path);
					auto FileState = MakeShared<FForgeSourceControlState>(Path);
					FileState->FileState = State;
					StateCache.Add(Path, FileState);
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
					FString LockPath = (*LockObj)->GetStringField(TEXT("path"));
					FString LockOwner = (*LockObj)->GetStringField(TEXT("owner"));
					FString FullPath = FPaths::ConvertRelativePathToFull(ProjDir, LockPath);
					FPaths::NormalizeFilename(FullPath);
					auto FileState = MakeShared<FForgeSourceControlState>(FullPath);
					FileState->FileState = FForgeSourceControlState::EFileState::Locked;
					FileState->LockOwner = LockOwner;
					StateCache.Add(FullPath, FileState);
				}
			}
		}
	}
}

ECommandResult::Type FForgeSourceControlProvider::GetState(
	const TArray<FString>& InFiles,
	TArray<FSourceControlStateRef>& OutState,
	EStateCacheUsage::Type InStateCacheUsage)
{
	// Never spawn a process here — always use cache.
	// Cache is refreshed explicitly after user actions (add, commit, etc.).

	// Return cached states for requested files.
	for (const FString& File : InFiles)
	{
		if (const FSourceControlStateRef* CachedState = StateCache.Find(File))
		{
			OutState.Add(*CachedState);
		}
		else
		{
			// File not in cache — assume unmodified/tracked.
			auto State = MakeShared<FForgeSourceControlState>(File);
			State->FileState = FForgeSourceControlState::EFileState::Unmodified;
			OutState.Add(State);
		}
	}

	return ECommandResult::Succeeded;
}

ECommandResult::Type FForgeSourceControlProvider::GetState(
	const TArray<FSourceControlChangelistRef>& InChangelists,
	TArray<FSourceControlChangelistStateRef>& OutState,
	EStateCacheUsage::Type InStateCacheUsage)
{
	// Forge doesn't use changelists — return empty.
	return ECommandResult::Succeeded;
}

TArray<FSourceControlStateRef> FForgeSourceControlProvider::GetCachedStateByPredicate(
	TFunctionRef<bool(const FSourceControlStateRef&)> Predicate) const
{
	TArray<FSourceControlStateRef> Result;
	for (const auto& Pair : StateCache)
	{
		if (Predicate(Pair.Value))
		{
			Result.Add(Pair.Value);
		}
	}
	return Result;
}

FDelegateHandle FForgeSourceControlProvider::RegisterSourceControlStateChanged_Handle(
	const FSourceControlStateChanged::FDelegate& SourceControlStateChanged)
{
	return OnSourceControlStateChanged.Add(SourceControlStateChanged);
}

void FForgeSourceControlProvider::UnregisterSourceControlStateChanged_Handle(FDelegateHandle Handle)
{
	OnSourceControlStateChanged.Remove(Handle);
}

ECommandResult::Type FForgeSourceControlProvider::Execute(
	const FSourceControlOperationRef& InOperation,
	FSourceControlChangelistPtr InChangelist,
	const TArray<FString>& InFiles,
	EConcurrency::Type InConcurrency,
	const FSourceControlOperationComplete& InOperationCompleteDelegate)
{
	ECommandResult::Type Result = ECommandResult::Failed;
	const FName OperationName = InOperation->GetName();

	if (OperationName == "Connect")
	{
		// Already verified in Init() — .forge directory exists.
		Result = bIsAvailable ? ECommandResult::Succeeded : ECommandResult::Failed;
	}
	else if (OperationName == "CheckOut")
	{
		// CheckOut = Lock in Forge (Perforce-style).
		for (const FString& File : InFiles)
		{
			FString RelPath = File;
			FPaths::MakePathRelativeTo(RelPath, *FPaths::ProjectDir());
			TSharedPtr<FJsonObject> JsonResult;
			if (RunForgeCommand(FString::Printf(TEXT("lock \"%s\""), *RelPath), JsonResult))
			{
				auto State = MakeShared<FForgeSourceControlState>(File);
				State->FileState = FForgeSourceControlState::EFileState::Locked;
				StateCache.Add(File, State);
			}
		}
		Result = ECommandResult::Succeeded;
	}
	else if (OperationName == "CheckIn")
	{
		// CheckIn = Snapshot + Unlock + Push.
		FString Message = InOperation->GetInProgressString().ToString();
		if (Message.IsEmpty()) { Message = TEXT("Checked in from Unreal Editor"); }

		// Add files.
		for (const FString& File : InFiles)
		{
			FString RelPath = File;
			FPaths::MakePathRelativeTo(RelPath, *FPaths::ProjectDir());
			TSharedPtr<FJsonObject> Unused;
			RunForgeCommand(FString::Printf(TEXT("add \"%s\""), *RelPath), Unused);
		}

		// Create snapshot.
		TSharedPtr<FJsonObject> Unused;
		RunForgeCommand(FString::Printf(TEXT("snapshot -m \"%s\""), *Message), Unused);

		// Unlock files.
		for (const FString& File : InFiles)
		{
			FString RelPath = File;
			FPaths::MakePathRelativeTo(RelPath, *FPaths::ProjectDir());
			RunForgeCommand(FString::Printf(TEXT("unlock \"%s\""), *RelPath), Unused);
		}

		// Push.
		RunForgeCommand(TEXT("push"), Unused);

		Result = ECommandResult::Succeeded;
	}
	else if (OperationName == "MarkForAdd")
	{
		for (const FString& File : InFiles)
		{
			FString RelPath = File;
			FPaths::MakePathRelativeTo(RelPath, *FPaths::ProjectDir());
			TSharedPtr<FJsonObject> Unused;
			RunForgeCommand(FString::Printf(TEXT("add \"%s\""), *RelPath), Unused);
		}
		Result = ECommandResult::Succeeded;
	}
	else if (OperationName == "UpdateStatus")
	{
		// Return success — cache is refreshed after explicit user actions only.
		Result = ECommandResult::Succeeded;
	}
	else if (OperationName == "Revert")
	{
		// TODO: restore files from latest snapshot, unlock
		Result = ECommandResult::Succeeded;
	}
	else if (OperationName == "Sync")
	{
		TSharedPtr<FJsonObject> Unused;
		Result = RunForgeCommand(TEXT("pull"), Unused)
			? ECommandResult::Succeeded
			: ECommandResult::Failed;
	}

	// Refresh cache after mutating operations.
	if (Result == ECommandResult::Succeeded &&
		(OperationName == "CheckOut" || OperationName == "CheckIn" ||
		 OperationName == "MarkForAdd" || OperationName == "Revert" || OperationName == "Sync"))
	{
		RefreshStatusCache();
	}

	// Notify completion.
	InOperationCompleteDelegate.ExecuteIfBound(InOperation, Result);
	return Result;
}

bool FForgeSourceControlProvider::CanCancelOperation(const FSourceControlOperationRef& InOperation) const
{
	return false;
}

void FForgeSourceControlProvider::CancelOperation(const FSourceControlOperationRef& InOperation)
{
}

void FForgeSourceControlProvider::Tick()
{
}

TArray<TSharedRef<class ISourceControlLabel>> FForgeSourceControlProvider::GetLabels(const FString& InMatchingSpec) const
{
	return TArray<TSharedRef<ISourceControlLabel>>();
}

TArray<FSourceControlChangelistRef> FForgeSourceControlProvider::GetChangelists(EStateCacheUsage::Type InStateCacheUsage)
{
	return TArray<FSourceControlChangelistRef>();
}

TSharedRef<class SWidget> FForgeSourceControlProvider::MakeSettingsWidget() const
{
	return SNullWidget::NullWidget;
}

bool FForgeSourceControlProvider::RunForgeCommand(const FString& Args, TSharedPtr<FJsonObject>& OutResult) const
{
	int32 ReturnCode = -1;
	FString StdOut;
	FString StdErr;

	FString FullArgs = FString::Printf(TEXT("--json %s"), *Args);

	// Run from the project directory so forge can find the .forge workspace.
	const FString ProjectDir = FPaths::ConvertRelativePathToFull(FPaths::ProjectDir());
	UE_LOG(LogSourceControl, Log, TEXT("Forge: running from dir '%s': forge %s"), *ProjectDir, *Args);

	FPlatformProcess::ExecProcess(
		*ForgeExePath,
		*FullArgs,
		&ReturnCode,
		&StdOut,
		&StdErr,
		*ProjectDir
	);

	if (ReturnCode != 0)
	{
		UE_LOG(LogSourceControl, Warning,
			TEXT("Forge command failed (exit %d): forge %s\n%s"),
			ReturnCode, *Args, *StdErr);
		return false;
	}

	// Parse JSON output.
	if (!StdOut.IsEmpty())
	{
		TSharedRef<TJsonReader<>> Reader = TJsonReaderFactory<>::Create(StdOut);
		if (!FJsonSerializer::Deserialize(Reader, OutResult) || !OutResult.IsValid())
		{
			UE_LOG(LogSourceControl, Warning,
				TEXT("Forge: failed to parse JSON output from: forge %s"), *Args);
			return false;
		}
	}

	return true;
}

#undef LOCTEXT_NAMESPACE
