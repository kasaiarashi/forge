// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#include "ForgeSourceControlProvider.h"
#include "ForgeSourceControlCommand.h"
#include "ForgeSourceControlWorkers.h"
#include "ForgeSourceControlModule.h"
#include "ISourceControlModule.h"
#include "SourceControlHelpers.h"
#include "SourceControlOperations.h"
#include "ScopedSourceControlProgress.h"
#include "Misc/Paths.h"
#include "Misc/FileHelper.h"
#include "Misc/QueuedThreadPool.h"
#include "Serialization/JsonReader.h"
#include "Serialization/JsonSerializer.h"
#include "HAL/PlatformProcess.h"
#include "Widgets/SBoxPanel.h"
#include "Widgets/Input/SButton.h"
#include "Widgets/Input/SEditableTextBox.h"
#include "Widgets/Layout/SBox.h"
#include "Widgets/Layout/SSeparator.h"
#include "Widgets/Notifications/SNotificationList.h"
#include "Widgets/Text/STextBlock.h"
#include "Framework/Notifications/NotificationManager.h"

#define LOCTEXT_NAMESPACE "ForgeSourceControl"

// ── Init / Close ────────────────────────────────────────────────────────────

void FForgeSourceControlProvider::Init(bool bForceConnection)
{
	ForgeExePath = TEXT("forge");
	bIsAvailable = false;

	// Walk up from project dir to find .forge/.
	FString SearchDir = FPaths::ConvertRelativePathToFull(FPaths::ProjectDir());
	FPaths::NormalizeDirectoryName(SearchDir);

	while (!SearchDir.IsEmpty())
	{
		const FString ForgeDir = SearchDir / TEXT(".forge");
		if (FPaths::DirectoryExists(ForgeDir))
		{
			WorkspaceRoot = SearchDir;
			bIsAvailable = true;
			UE_LOG(LogSourceControl, Log, TEXT("Forge: workspace root at %s"), *WorkspaceRoot);

			// Read user name from .forge/config.json.
			const FString ConfigPath = ForgeDir / TEXT("config.json");
			FString ConfigJson;
			if (FFileHelper::LoadFileToString(ConfigJson, *ConfigPath))
			{
				TSharedPtr<FJsonObject> ConfigObj;
				TSharedRef<TJsonReader<>> Reader = TJsonReaderFactory<>::Create(ConfigJson);
				if (FJsonSerializer::Deserialize(Reader, ConfigObj) && ConfigObj.IsValid())
				{
					const TSharedPtr<FJsonObject>* UserObj;
					if (ConfigObj->TryGetObjectField(TEXT("user"), UserObj))
					{
						(*UserObj)->TryGetStringField(TEXT("name"), CurrentUserName);
						UE_LOG(LogSourceControl, Log, TEXT("Forge: user is '%s'"), *CurrentUserName);
					}
				}
			}
			break;
		}

		FString Parent = FPaths::GetPath(SearchDir);
		if (Parent == SearchDir || Parent.IsEmpty()) break;
		SearchDir = Parent;
	}
}

void FForgeSourceControlProvider::Close()
{
	bIsAvailable = false;
	WorkspaceRoot.Empty();
	CurrentUserName.Empty();
	StateCache.Empty();

	for (FForgeSourceControlCommand* Cmd : CommandQueue)
	{
		Cmd->Abandon();
		if (Cmd->bAutoDelete) delete Cmd;
	}
	CommandQueue.Empty();
}

// ── Identity ────────────────────────────────────────────────────────────────

const FName& FForgeSourceControlProvider::GetName() const
{
	static const FName ProviderName("Forge");
	return ProviderName;
}

FText FForgeSourceControlProvider::GetStatusText() const
{
	return bIsAvailable
		? LOCTEXT("StatusAvailable", "Connected to Forge workspace")
		: LOCTEXT("StatusUnavailable", "No Forge workspace found");
}

TMap<ISourceControlProvider::EStatus, FString> FForgeSourceControlProvider::GetStatus() const
{
	TMap<EStatus, FString> Result;
	Result.Add(EStatus::Enabled, IsEnabled() ? TEXT("Yes") : TEXT("No"));
	Result.Add(EStatus::Connected, IsAvailable() ? TEXT("Yes") : TEXT("No"));
	return Result;
}

bool FForgeSourceControlProvider::IsEnabled() const { return true; }
bool FForgeSourceControlProvider::IsAvailable() const { return bIsAvailable; }

// ── State cache ─────────────────────────────────────────────────────────────

TSharedRef<FForgeSourceControlState> FForgeSourceControlProvider::GetStateInternal(const FString& Filename)
{
	if (TSharedRef<FForgeSourceControlState>* Existing = StateCache.Find(Filename))
	{
		return *Existing;
	}

	TSharedRef<FForgeSourceControlState> NewState = MakeShared<FForgeSourceControlState>(Filename);
	NewState->FileState = FForgeSourceControlState::EFileState::Unmodified;
	StateCache.Add(Filename, NewState);
	return NewState;
}

bool FForgeSourceControlProvider::UpdateCachedStates(
	const TMap<FString, FForgeSourceControlState::EFileState>& InStates,
	const TMap<FString, FString>& InLockOwners)
{
	// Collect which files are in the new status snapshot.
	TSet<FString> NewPaths;
	NewPaths.Reserve(InStates.Num());

	// Apply new states (create-or-update, never remove).
	for (const auto& Pair : InStates)
	{
		NewPaths.Add(Pair.Key);
		TSharedRef<FForgeSourceControlState> State = GetStateInternal(Pair.Key);
		State->FileState = Pair.Value;

		if (const FString* Owner = InLockOwners.Find(Pair.Key))
		{
			State->LockOwner = *Owner;
		}
		else
		{
			State->LockOwner = FString();
		}
	}

	// Files in cache but NOT in new snapshot → reset to Unmodified.
	// Only touch the enum (uint8, safe); leave FString members untouched.
	for (auto& Pair : StateCache)
	{
		if (!NewPaths.Contains(Pair.Key))
		{
			Pair.Value->FileState = FForgeSourceControlState::EFileState::Unmodified;
		}
	}

	return InStates.Num() > 0;
}

void FForgeSourceControlProvider::RefreshStatusAsync()
{
	if (!bIsAvailable) return;
	auto Operation = ISourceControlOperation::Create<FUpdateStatus>();
	Execute(Operation, nullptr, TArray<FString>(), EConcurrency::Asynchronous,
		FSourceControlOperationComplete());
}

ECommandResult::Type FForgeSourceControlProvider::GetState(
	const TArray<FString>& InFiles,
	TArray<FSourceControlStateRef>& OutState,
	EStateCacheUsage::Type InStateCacheUsage)
{
	TArray<FString> AbsoluteFiles = SourceControlHelpers::AbsoluteFilenames(InFiles);

	if (InStateCacheUsage == EStateCacheUsage::ForceUpdate)
	{
		RefreshStatusAsync();
	}

	for (const FString& File : AbsoluteFiles)
	{
		OutState.Add(GetStateInternal(File));
	}

	return ECommandResult::Succeeded;
}

ECommandResult::Type FForgeSourceControlProvider::GetState(
	const TArray<FSourceControlChangelistRef>& InChangelists,
	TArray<FSourceControlChangelistStateRef>& OutState,
	EStateCacheUsage::Type InStateCacheUsage)
{
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

TOptional<int> FForgeSourceControlProvider::GetNumLocalChanges() const
{
	int32 Count = 0;
	for (const auto& Pair : StateCache)
	{
		if (Pair.Value->FileState != FForgeSourceControlState::EFileState::Unmodified &&
			Pair.Value->FileState != FForgeSourceControlState::EFileState::Untracked)
		{
			Count++;
		}
	}
	return Count;
}

// ── Delegates ───────────────────────────────────────────────────────────────

FDelegateHandle FForgeSourceControlProvider::RegisterSourceControlStateChanged_Handle(
	const FSourceControlStateChanged::FDelegate& SourceControlStateChanged)
{
	return OnSourceControlStateChanged.Add(SourceControlStateChanged);
}

void FForgeSourceControlProvider::UnregisterSourceControlStateChanged_Handle(FDelegateHandle Handle)
{
	OnSourceControlStateChanged.Remove(Handle);
}

// ── Worker registration ─────────────────────────────────────────────────────

void FForgeSourceControlProvider::RegisterWorker(const FName& InName, const FGetForgeWorker& InDelegate)
{
	WorkersMap.Add(InName, InDelegate);
}

TSharedPtr<IForgeWorker, ESPMode::ThreadSafe> FForgeSourceControlProvider::CreateWorker(const FName& InOperationName) const
{
	const FGetForgeWorker* Delegate = WorkersMap.Find(InOperationName);
	if (Delegate != nullptr)
	{
		return Delegate->Execute();
	}
	return nullptr;
}

bool FForgeSourceControlProvider::CanExecuteOperation(const FSourceControlOperationRef& InOperation) const
{
	return WorkersMap.Find(InOperation->GetName()) != nullptr;
}

// ── Execute ─────────────────────────────────────────────────────────────────

ECommandResult::Type FForgeSourceControlProvider::Execute(
	const FSourceControlOperationRef& InOperation,
	FSourceControlChangelistPtr InChangelist,
	const TArray<FString>& InFiles,
	EConcurrency::Type InConcurrency,
	const FSourceControlOperationComplete& InOperationCompleteDelegate)
{
	UE_LOG(LogSourceControl, Log, TEXT("Forge: Execute '%s' (%s, %d files)"),
		*InOperation->GetName().ToString(),
		InConcurrency == EConcurrency::Synchronous ? TEXT("sync") : TEXT("async"),
		InFiles.Num());

	const TArray<FString> AbsoluteFiles = SourceControlHelpers::AbsoluteFilenames(InFiles);

	TSharedPtr<IForgeWorker, ESPMode::ThreadSafe> Worker = CreateWorker(InOperation->GetName());
	if (!Worker.IsValid())
	{
		UE_LOG(LogSourceControl, Warning, TEXT("Forge: operation '%s' not supported"),
			*InOperation->GetName().ToString());
		InOperationCompleteDelegate.ExecuteIfBound(InOperation, ECommandResult::Failed);
		return ECommandResult::Failed;
	}

	FForgeSourceControlCommand* Command = new FForgeSourceControlCommand(
		InOperation, Worker.ToSharedRef(), InConcurrency, *this);
	Command->Files = AbsoluteFiles;
	Command->OperationCompleteDelegate = InOperationCompleteDelegate;

	if (InConcurrency == EConcurrency::Synchronous)
	{
		Command->bAutoDelete = false;
		return ExecuteSynchronousCommand(*Command, InOperation->GetInProgressString());
	}
	else
	{
		Command->bAutoDelete = true;
		return IssueCommand(*Command);
	}
}

// ── Command dispatch ────────────────────────────────────────────────────────

ECommandResult::Type FForgeSourceControlProvider::IssueCommand(FForgeSourceControlCommand& InCommand)
{
	if (GThreadPool != nullptr)
	{
		GThreadPool->AddQueuedWork(&InCommand);
		CommandQueue.Add(&InCommand);
		return ECommandResult::Succeeded;
	}

	UE_LOG(LogSourceControl, Error, TEXT("Forge: no thread pool available"));
	InCommand.MarkOperationCompleted(false);
	return ECommandResult::Failed;
}

ECommandResult::Type FForgeSourceControlProvider::ExecuteSynchronousCommand(
	FForgeSourceControlCommand& InCommand, const FText& Task)
{
	ECommandResult::Type Result = ECommandResult::Failed;

	{
		FScopedSourceControlProgress Progress(Task);
		IssueCommand(InCommand);

		while (!InCommand.bExecuteProcessed)
		{
			Tick();
			Progress.Tick();
			FPlatformProcess::Sleep(0.01f);
		}

		Tick(); // Process the completed command.

		if (InCommand.bCommandSuccessful)
		{
			Result = ECommandResult::Succeeded;
		}
	}

	if (CommandQueue.Contains(&InCommand))
	{
		CommandQueue.Remove(&InCommand);
	}
	delete &InCommand;

	return Result;
}

// ── Tick ─────────────────────────────────────────────────────────────────────

void FForgeSourceControlProvider::Tick()
{
	// Broadcast deferred from previous tick (gives renderer a full frame to finish).
	if (bPendingBroadcast)
	{
		bPendingBroadcast = false;
		OnSourceControlStateChanged.Broadcast();
	}

	for (int32 i = 0; i < CommandQueue.Num(); ++i)
	{
		FForgeSourceControlCommand& Command = *CommandQueue[i];
		if (Command.bExecuteProcessed)
		{
			UE_LOG(LogSourceControl, Log, TEXT("Forge: completed '%s' (success=%d)"),
				*Command.Worker->GetName().ToString(), Command.bCommandSuccessful);

			CommandQueue.RemoveAt(i);

			if (Command.Worker->UpdateStates())
			{
				bPendingBroadcast = true; // Broadcast on NEXT tick.
			}

			Command.ReturnResults();

			if (Command.bAutoDelete)
			{
				delete &Command;
			}
			break; // One per tick.
		}
	}
}

// ── Stubs ───────────────────────────────────────────────────────────────────

bool FForgeSourceControlProvider::CanCancelOperation(const FSourceControlOperationRef& InOperation) const { return false; }
void FForgeSourceControlProvider::CancelOperation(const FSourceControlOperationRef& InOperation) {}

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
	FForgeSourceControlProvider* MutableThis = const_cast<FForgeSourceControlProvider*>(this);

	// Pre-fill user name from the OS so the common case ("accept defaults")
	// actually works without typing anything.
	TSharedRef<FString> RemoteUrlRef = MakeShared<FString>();
	TSharedRef<FString> UserNameRef = MakeShared<FString>(FPlatformProcess::UserName());
	TSharedRef<FString> UserEmailRef = MakeShared<FString>();

	TSharedRef<SVerticalBox> Root = SNew(SVerticalBox);

	// ── Forge executable path (always shown) ────────────────────────────────
	Root->AddSlot()
		.AutoHeight()
		.Padding(2.0f)
		[
			SNew(SHorizontalBox)
			+ SHorizontalBox::Slot()
			.AutoWidth()
			.VAlign(VAlign_Center)
			[
				SNew(STextBlock).Text(LOCTEXT("ForgePathLabel", "Forge Executable Path:"))
			]
			+ SHorizontalBox::Slot()
			.FillWidth(1.0f)
			.Padding(4.0f, 0.0f)
			[
				SNew(SEditableTextBox)
				.Text(FText::FromString(ForgeExePath))
				.OnTextCommitted_Lambda([MutableThis](const FText& NewText, ETextCommit::Type)
				{
					MutableThis->ForgeExePath = NewText.ToString();
				})
			]
		];

	// ── Init section: only shown when no workspace has been detected ────────
	if (!bIsAvailable)
	{
		Root->AddSlot()
			.AutoHeight()
			.Padding(2.0f, 8.0f, 2.0f, 2.0f)
			[
				SNew(SSeparator)
			];

		Root->AddSlot()
			.AutoHeight()
			.Padding(2.0f)
			[
				SNew(STextBlock)
				.Text(LOCTEXT("InitHeader", "No Forge workspace detected for this project."))
				.AutoWrapText(true)
			];

		auto MakeRow = [](const FText& Label, TSharedRef<FString> Target, const FText& Hint)
		{
			return SNew(SHorizontalBox)
				+ SHorizontalBox::Slot()
				.AutoWidth()
				.VAlign(VAlign_Center)
				.Padding(0.0f, 0.0f, 4.0f, 0.0f)
				[
					SNew(SBox)
					.WidthOverride(110.0f)
					[
						SNew(STextBlock).Text(Label)
					]
				]
				+ SHorizontalBox::Slot()
				.FillWidth(1.0f)
				[
					SNew(SEditableTextBox)
					.Text(FText::FromString(*Target))
					.HintText(Hint)
					.OnTextChanged_Lambda([Target](const FText& NewText)
					{
						*Target = NewText.ToString();
					})
				];
		};

		Root->AddSlot()
			.AutoHeight()
			.Padding(2.0f, 4.0f)
			[
				MakeRow(
					LOCTEXT("UserNameLabel", "User Name:"),
					UserNameRef,
					LOCTEXT("UserNameHint", "Your display name"))
			];

		Root->AddSlot()
			.AutoHeight()
			.Padding(2.0f, 4.0f)
			[
				MakeRow(
					LOCTEXT("UserEmailLabel", "User Email:"),
					UserEmailRef,
					LOCTEXT("UserEmailHint", "you@example.com (optional)"))
			];

		Root->AddSlot()
			.AutoHeight()
			.Padding(2.0f, 4.0f)
			[
				MakeRow(
					LOCTEXT("RemoteUrlLabel", "Remote URL:"),
					RemoteUrlRef,
					LOCTEXT("RemoteUrlHint", "https://server/owner/repo (optional)"))
			];

		Root->AddSlot()
			.AutoHeight()
			.Padding(2.0f, 8.0f)
			.HAlign(HAlign_Left)
			[
				SNew(SButton)
				.Text(LOCTEXT("InitButton", "Initialize Project with Forge"))
				.ToolTipText(LOCTEXT("InitButtonTooltip",
					"Creates a .forge workspace in the project directory, sets your user info, "
					"and (if provided) adds an 'origin' remote."))
				.OnClicked_Lambda([MutableThis, RemoteUrlRef, UserNameRef, UserEmailRef]()
				{
					FText Error;
					const bool bOk = MutableThis->InitializeWorkspace(
						*RemoteUrlRef, *UserNameRef, *UserEmailRef, Error);

					FNotificationInfo Info(bOk
						? LOCTEXT("InitOk", "Forge workspace initialized.")
						: FText::Format(LOCTEXT("InitFailFmt", "Forge init failed: {0}"), Error));
					Info.ExpireDuration = bOk ? 4.0f : 8.0f;
					FSlateNotificationManager::Get().AddNotification(Info);

					if (bOk)
					{
						MutableThis->RefreshStatusAsync();
					}
					return FReply::Handled();
				})
			];
	}

	return Root;
}

// ── CLI execution (thread-safe) ─────────────────────────────────────────────

bool FForgeSourceControlProvider::RunForgeCommand(const FString& Args, TSharedPtr<FJsonObject>& OutResult) const
{
	int32 ReturnCode = -1;
	FString StdOut, StdErr;
	FString FullArgs = FString::Printf(TEXT("--json %s"), *Args);

	UE_LOG(LogSourceControl, Log, TEXT("Forge: forge --json %s"), *Args);

	FPlatformProcess::ExecProcess(
		*ForgeExePath, *FullArgs,
		&ReturnCode, &StdOut, &StdErr, *WorkspaceRoot);

	if (ReturnCode != 0)
	{
		UE_LOG(LogSourceControl, Warning, TEXT("Forge: exit %d: forge %s\n%s"),
			ReturnCode, *Args, *StdErr);
		return false;
	}

	if (!StdOut.IsEmpty())
	{
		TSharedRef<TJsonReader<>> Reader = TJsonReaderFactory<>::Create(StdOut);
		if (!FJsonSerializer::Deserialize(Reader, OutResult) || !OutResult.IsValid())
		{
			UE_LOG(LogSourceControl, Warning, TEXT("Forge: bad JSON from: forge %s"), *Args);
			return false;
		}
	}

	return true;
}

bool FForgeSourceControlProvider::RunForgeCommandRaw(const FString& Args) const
{
	int32 ReturnCode = -1;
	FString StdOut, StdErr;

	UE_LOG(LogSourceControl, Log, TEXT("Forge: forge %s"), *Args);

	FPlatformProcess::ExecProcess(
		*ForgeExePath, *Args,
		&ReturnCode, &StdOut, &StdErr, *WorkspaceRoot);

	if (ReturnCode != 0)
	{
		UE_LOG(LogSourceControl, Warning, TEXT("Forge: exit %d: forge %s\n%s"),
			ReturnCode, *Args, *StdErr);
		return false;
	}

	return true;
}

bool FForgeSourceControlProvider::RunForgeCommandInDir(
	const FString& Args, const FString& Dir, FString& OutStdErr) const
{
	int32 ReturnCode = -1;
	FString StdOut;

	UE_LOG(LogSourceControl, Log, TEXT("Forge: (cwd=%s) forge %s"), *Dir, *Args);

	FPlatformProcess::ExecProcess(
		*ForgeExePath, *Args,
		&ReturnCode, &StdOut, &OutStdErr, *Dir);

	if (ReturnCode != 0)
	{
		UE_LOG(LogSourceControl, Warning, TEXT("Forge: exit %d: forge %s\n%s"),
			ReturnCode, *Args, *OutStdErr);
		return false;
	}
	return true;
}

// ── Workspace bootstrap ─────────────────────────────────────────────────────

bool FForgeSourceControlProvider::InitializeWorkspace(
	const FString& RemoteUrl,
	const FString& UserName,
	const FString& UserEmail,
	FText& OutError)
{
	const FString ProjectDir = FPaths::ConvertRelativePathToFull(FPaths::ProjectDir());
	if (!FPaths::DirectoryExists(ProjectDir))
	{
		OutError = LOCTEXT("InitErrNoDir", "Project directory does not exist.");
		return false;
	}

	// Block re-init if a .forge dir is already present — the CLI would error
	// anyway, but a clear message up-front is friendlier than surfacing stderr.
	if (FPaths::DirectoryExists(ProjectDir / TEXT(".forge")))
	{
		OutError = LOCTEXT("InitErrAlreadyInit", "This project already has a .forge workspace.");
		return false;
	}

	FString StdErr;

	// 1. forge init
	if (!RunForgeCommandInDir(TEXT("init"), ProjectDir, StdErr))
	{
		OutError = FText::FromString(StdErr.IsEmpty() ? TEXT("forge init failed") : StdErr);
		return false;
	}

	// 2. user.name / user.email — only set if non-empty so we don't overwrite
	//    the CLI's whoami-derived default with a blank string.
	if (!UserName.IsEmpty())
	{
		const FString Args = FString::Printf(TEXT("config set user.name \"%s\""), *UserName);
		if (!RunForgeCommandInDir(Args, ProjectDir, StdErr))
		{
			OutError = FText::FromString(StdErr.IsEmpty() ? TEXT("failed to set user.name") : StdErr);
			return false;
		}
	}
	if (!UserEmail.IsEmpty())
	{
		const FString Args = FString::Printf(TEXT("config set user.email \"%s\""), *UserEmail);
		if (!RunForgeCommandInDir(Args, ProjectDir, StdErr))
		{
			OutError = FText::FromString(StdErr.IsEmpty() ? TEXT("failed to set user.email") : StdErr);
			return false;
		}
	}

	// 3. remote add origin — optional; skip entirely if user didn't provide one.
	if (!RemoteUrl.IsEmpty())
	{
		const FString Args = FString::Printf(TEXT("remote add origin \"%s\""), *RemoteUrl);
		if (!RunForgeCommandInDir(Args, ProjectDir, StdErr))
		{
			OutError = FText::FromString(StdErr.IsEmpty() ? TEXT("failed to add remote") : StdErr);
			return false;
		}
	}

	// Re-run Init() so WorkspaceRoot / CurrentUserName / bIsAvailable pick up
	// the freshly-created .forge without an editor restart.
	Init(false);
	return true;
}

#undef LOCTEXT_NAMESPACE
