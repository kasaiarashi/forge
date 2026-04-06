// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#include "ForgeSourceControlProvider.h"
#include "ForgeSourceControlState.h"
#include "ISourceControlModule.h"
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

	// Check if we're inside a forge workspace by running `forge status --json`.
	TSharedPtr<FJsonObject> Result;
	bIsAvailable = RunForgeCommand(TEXT("status --json"), Result);
}

void FForgeSourceControlProvider::Close()
{
	bIsAvailable = false;
	StateCache.Empty();
}

FName FForgeSourceControlProvider::GetName() const
{
	return FName("Forge");
}

FText FForgeSourceControlProvider::GetStatusText() const
{
	if (bIsAvailable)
	{
		return LOCTEXT("StatusAvailable", "Connected to Forge workspace");
	}
	return LOCTEXT("StatusUnavailable", "No Forge workspace found");
}

bool FForgeSourceControlProvider::IsEnabled() const
{
	return true;
}

bool FForgeSourceControlProvider::IsAvailable() const
{
	return bIsAvailable;
}

ECommandResult::Type FForgeSourceControlProvider::GetState(
	const TArray<FString>& InFiles,
	TArray<FSourceControlStateRef>& OutState,
	EStateCacheUsage::Type InStateCacheUsage)
{
	if (InStateCacheUsage == EStateCacheUsage::ForceUpdate)
	{
		// Run forge status --json to refresh.
		TSharedPtr<FJsonObject> Result;
		if (RunForgeCommand(TEXT("status --json"), Result) && Result.IsValid())
		{
			// Parse staged files.
			const TArray<TSharedPtr<FJsonValue>>* StagedFiles;
			if (Result->TryGetArrayField(TEXT("staged"), StagedFiles))
			{
				for (const auto& Val : *StagedFiles)
				{
					FString Path = FPaths::ConvertRelativePathToFull(
						FPaths::ProjectDir(), Val->AsString());
					auto State = MakeShared<FForgeSourceControlState>(Path);
					State->FileState = FForgeSourceControlState::EFileState::Added;
					StateCache.Add(Path, State);
				}
			}

			// Parse modified files.
			const TArray<TSharedPtr<FJsonValue>>* ModifiedFiles;
			if (Result->TryGetArrayField(TEXT("modified"), ModifiedFiles))
			{
				for (const auto& Val : *ModifiedFiles)
				{
					FString Path = FPaths::ConvertRelativePathToFull(
						FPaths::ProjectDir(), Val->AsString());
					auto State = MakeShared<FForgeSourceControlState>(Path);
					State->FileState = FForgeSourceControlState::EFileState::Modified;
					StateCache.Add(Path, State);
				}
			}

			// Parse deleted files.
			const TArray<TSharedPtr<FJsonValue>>* DeletedFiles;
			if (Result->TryGetArrayField(TEXT("deleted"), DeletedFiles))
			{
				for (const auto& Val : *DeletedFiles)
				{
					FString Path = FPaths::ConvertRelativePathToFull(
						FPaths::ProjectDir(), Val->AsString());
					auto State = MakeShared<FForgeSourceControlState>(Path);
					State->FileState = FForgeSourceControlState::EFileState::Deleted;
					StateCache.Add(Path, State);
				}
			}

			// Parse untracked files.
			const TArray<TSharedPtr<FJsonValue>>* UntrackedFiles;
			if (Result->TryGetArrayField(TEXT("untracked"), UntrackedFiles))
			{
				for (const auto& Val : *UntrackedFiles)
				{
					FString Path = FPaths::ConvertRelativePathToFull(
						FPaths::ProjectDir(), Val->AsString());
					auto State = MakeShared<FForgeSourceControlState>(Path);
					State->FileState = FForgeSourceControlState::EFileState::Untracked;
					StateCache.Add(Path, State);
				}
			}
		}
	}

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
		TSharedPtr<FJsonObject> JsonResult;
		Result = RunForgeCommand(TEXT("status --json"), JsonResult)
			? ECommandResult::Succeeded
			: ECommandResult::Failed;
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
		TArray<FSourceControlStateRef> States;
		Result = GetState(InFiles, States, EStateCacheUsage::ForceUpdate);
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

#if SOURCE_CONTROL_WITH_SLATE
TSharedRef<class SWidget> FForgeSourceControlProvider::MakeSettingsWidget() const
{
	return SNullWidget::NullWidget;
}
#endif

bool FForgeSourceControlProvider::RunForgeCommand(const FString& Args, TSharedPtr<FJsonObject>& OutResult) const
{
	int32 ReturnCode = -1;
	FString StdOut;
	FString StdErr;

	FString FullArgs = FString::Printf(TEXT("--json %s"), *Args);

	FPlatformProcess::ExecProcess(
		*ForgeExePath,
		*FullArgs,
		&ReturnCode,
		&StdOut,
		&StdErr
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
