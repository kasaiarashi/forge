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

	virtual FName GetName() const override;
	virtual FText GetStatusText() const override;

	virtual bool IsEnabled() const override;
	virtual bool IsAvailable() const override;

	virtual ECommandResult::Type GetState(
		const TArray<FString>& InFiles,
		TArray<FSourceControlStateRef>& OutState,
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

	virtual bool CanCancelOperation(const FSourceControlOperationRef& InOperation) const override;
	virtual void CancelOperation(const FSourceControlOperationRef& InOperation) override;

	virtual void Tick() override;

	virtual TArray<TSharedRef<class ISourceControlLabel>> GetLabels(const FString& InMatchingSpec) const override;

#if SOURCE_CONTROL_WITH_SLATE
	virtual TSharedRef<class SWidget> MakeSettingsWidget() const override;
#endif

private:
	/** Run the forge CLI and return parsed JSON output. */
	bool RunForgeCommand(const FString& Args, TSharedPtr<FJsonObject>& OutResult) const;

	/** Path to the forge executable. */
	FString ForgeExePath;

	/** Whether we successfully connected to a forge workspace. */
	bool bIsAvailable = false;

	/** Cached file states. */
	TMap<FString, FSourceControlStateRef> StateCache;

	/** Delegates for state change notifications. */
	FSourceControlStateChanged OnSourceControlStateChanged;
};
