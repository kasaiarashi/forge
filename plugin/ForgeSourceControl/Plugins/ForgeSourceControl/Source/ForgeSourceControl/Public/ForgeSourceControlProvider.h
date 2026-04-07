// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#pragma once

#include "ISourceControlProvider.h"
#include "ISourceControlOperation.h"

/**
 * Forge source control provider.
 * Shells out to the `forge` CLI with --json for all operations.
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

	virtual bool CanExecuteOperation(const FSourceControlOperationRef& InOperation) const override { return true; }
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
	virtual bool UsesSnapshots() const override { return true; }
	virtual bool AllowsDiffAgainstDepot() const override { return false; }

	virtual TOptional<bool> IsAtLatestRevision() const override { return TOptional<bool>(); }
	virtual TOptional<int> GetNumLocalChanges() const override { return TOptional<int>(); }

	virtual TSharedRef<class SWidget> MakeSettingsWidget() const override;

	/** Explicitly refresh the status cache by running forge status --json. */
	void RefreshStatusCache();

private:
	/** Run the forge CLI and return parsed JSON output. */
	bool RunForgeCommand(const FString& Args, TSharedPtr<FJsonObject>& OutResult) const;

	/** Path to the forge executable. */
	FString ForgeExePath;

	/** Whether we successfully connected to a forge workspace. */
	bool bIsAvailable = false;

	/** Whether the cache needs a background refresh. */
	bool bNeedsCacheRefresh = false;

	/** Cached file states. */
	TMap<FString, FSourceControlStateRef> StateCache;

	/** Delegates for state change notifications. */
	FSourceControlStateChanged OnSourceControlStateChanged;
};
