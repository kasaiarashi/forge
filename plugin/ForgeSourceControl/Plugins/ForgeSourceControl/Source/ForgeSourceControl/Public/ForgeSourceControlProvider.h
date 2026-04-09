// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#pragma once

#include "ISourceControlProvider.h"
#include "ISourceControlOperation.h"
#include "ForgeSourceControlState.h"
#include "ForgeSourceControlWorkers.h"

class FForgeSourceControlCommand;

/**
 * Forge source control provider.
 *
 * Uses the Command/Worker/ThreadPool pattern:
 *   Execute() creates a command and dispatches it to GThreadPool.
 *   Worker::Execute() runs forge CLI on a background thread.
 *   Tick() processes completed commands on the game thread.
 */
class FForgeSourceControlProvider : public ISourceControlProvider
{
public:
	virtual void Init(bool bForceConnection = true) override;
	virtual void Close() override;

	virtual const FName& GetName() const override;
	virtual FText GetStatusText() const override;
	virtual TMap<EStatus, FString> GetStatus() const override;
	virtual bool IsEnabled() const override;
	virtual bool IsAvailable() const override;

	virtual bool QueryStateBranchConfig(const FString& ConfigSrc, const FString& ConfigDest) override { return false; }
	virtual void RegisterStateBranches(const TArray<FString>& BranchNames, const FString& ContentRoot) override {}
	virtual int32 GetStateBranchIndex(const FString& BranchName) const override { return INDEX_NONE; }
	virtual bool GetStateBranchAtIndex(int32 BranchIndex, FString& OutBranchName) const override { return false; }

	virtual ECommandResult::Type GetState(
		const TArray<FString>& InFiles,
		TArray<FSourceControlStateRef>& OutState,
		EStateCacheUsage::Type InStateCacheUsage) override;

	virtual ECommandResult::Type GetState(
		const TArray<FSourceControlChangelistRef>& InChangelists,
		TArray<FSourceControlChangelistStateRef>& OutState,
		EStateCacheUsage::Type InStateCacheUsage) override;

	virtual TArray<FSourceControlStateRef> GetCachedStateByPredicate(
		TFunctionRef<bool(const FSourceControlStateRef&)> Predicate) const override;

	virtual FDelegateHandle RegisterSourceControlStateChanged_Handle(
		const FSourceControlStateChanged::FDelegate& SourceControlStateChanged) override;
	virtual void UnregisterSourceControlStateChanged_Handle(FDelegateHandle Handle) override;

	virtual ECommandResult::Type Execute(
		const FSourceControlOperationRef& InOperation,
		FSourceControlChangelistPtr InChangelist,
		const TArray<FString>& InFiles,
		EConcurrency::Type InConcurrency,
		const FSourceControlOperationComplete& InOperationCompleteDelegate) override;

	virtual bool CanExecuteOperation(const FSourceControlOperationRef& InOperation) const override;
	virtual bool CanCancelOperation(const FSourceControlOperationRef& InOperation) const override;
	virtual void CancelOperation(const FSourceControlOperationRef& InOperation) override;

	virtual void Tick() override;

	virtual TArray<TSharedRef<class ISourceControlLabel>> GetLabels(const FString& InMatchingSpec) const override;
	virtual TArray<FSourceControlChangelistRef> GetChangelists(EStateCacheUsage::Type InStateCacheUsage) override;

	virtual bool UsesLocalReadOnlyState() const override { return false; }
	virtual bool UsesChangelists() const override { return false; }
	virtual bool UsesUncontrolledChangelists() const override { return false; }
	virtual bool UsesCheckout() const override { return true; }
	virtual bool UsesFileRevisions() const override { return false; }
	// Must be false: when true, FSourceControlWindows::PromptForCheckin auto-calls
	// SyncLatest() before checkin, which reloads every loaded package (including
	// engine packages) and crashes the render thread. Git/Diversion both return false.
	virtual bool UsesSnapshots() const override { return false; }
	virtual bool AllowsDiffAgainstDepot() const override { return false; }

	virtual TOptional<bool> IsAtLatestRevision() const override { return TOptional<bool>(); }
	virtual TOptional<int> GetNumLocalChanges() const override;

	virtual TSharedRef<class SWidget> MakeSettingsWidget() const override;

	// ── Public API for workers ──────────────────────────────────────────────

	void RegisterWorker(const FName& InName, const FGetForgeWorker& InDelegate);

	/** Get or create a cached state. Never invalidates existing TSharedRef pointers. */
	TSharedRef<FForgeSourceControlState> GetStateInternal(const FString& Filename);

	/** Update existing state objects in-place (never removes them from cache). */
	bool UpdateCachedStates(
		const TMap<FString, FForgeSourceControlState::EFileState>& InStates,
		const TMap<FString, FString>& InLockOwners);

	/** Queue an async UpdateStatus refresh. */
	void RefreshStatusAsync();

	/** CLI execution — thread-safe. */
	bool RunForgeCommand(const FString& Args, TSharedPtr<FJsonObject>& OutResult) const;
	bool RunForgeCommandRaw(const FString& Args) const;

	const FString& GetWorkspaceRoot() const { return WorkspaceRoot; }
	const FString& GetCurrentUserName() const { return CurrentUserName; }

private:
	TSharedPtr<IForgeWorker, ESPMode::ThreadSafe> CreateWorker(const FName& InOperationName) const;
	ECommandResult::Type IssueCommand(FForgeSourceControlCommand& InCommand);
	ECommandResult::Type ExecuteSynchronousCommand(FForgeSourceControlCommand& InCommand, const FText& Task);

	TMap<FName, FGetForgeWorker> WorkersMap;
	TArray<FForgeSourceControlCommand*> CommandQueue;

	/** State cache — objects are NEVER removed, only updated in-place. */
	TMap<FString, TSharedRef<FForgeSourceControlState>> StateCache;

	FString ForgeExePath;
	FString WorkspaceRoot;
	FString CurrentUserName;
	bool bIsAvailable = false;

	/** Deferred broadcast — set when states change, fires on the NEXT tick. */
	bool bPendingBroadcast = false;

	FSourceControlStateChanged OnSourceControlStateChanged;
};
