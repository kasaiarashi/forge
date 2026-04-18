// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

#pragma once

#include "ISourceControlState.h"
#include "ISourceControlRevision.h"

/**
 * The state of a single file under Forge source control.
 */
class FForgeSourceControlState : public ISourceControlState
{
public:
	FForgeSourceControlState(const FString& InFilename)
		: Filename(InFilename)
	{
	}

	// -- File state flags --

	enum class EFileState : uint8
	{
		Unmodified,
		Modified,
		Added,
		Deleted,
		Untracked,
		Locked,          // We hold the lock.
		LockedByOther,   // Someone else holds the lock.
	};

	/** The current state of this file. */
	EFileState FileState = EFileState::Unmodified;

	/** Who holds the lock (if any). */
	FString LockOwner;

	// -- ISourceControlState interface --

	virtual int32 GetHistorySize() const override { return History.Num(); }
	virtual TSharedPtr<class ISourceControlRevision, ESPMode::ThreadSafe> GetHistoryItem(int32 HistoryIndex) const override;
	virtual TSharedPtr<class ISourceControlRevision, ESPMode::ThreadSafe> FindHistoryRevision(int32 RevisionNumber) const override;
	virtual TSharedPtr<class ISourceControlRevision, ESPMode::ThreadSafe> FindHistoryRevision(const FString& InRevision) const override;

	virtual TSharedPtr<class ISourceControlRevision, ESPMode::ThreadSafe> GetCurrentRevision() const override { return nullptr; }

	virtual FSlateIcon GetIcon() const override;
	virtual FText GetDisplayName() const override;
	virtual FText GetDisplayTooltip() const override;
	virtual const FString& GetFilename() const override { return Filename; }
	virtual const FDateTime& GetTimeStamp() const override { return TimeStamp; }

	virtual bool CanCheckIn() const override;
	virtual bool CanCheckout() const override;
	virtual bool IsCheckedOut() const override;
	virtual bool IsCheckedOutOther(FString* Who = nullptr) const override;
	virtual bool IsCheckedOutInOtherBranch(const FString& CurrentBranch = FString()) const override { return false; }
	virtual bool IsModifiedInOtherBranch(const FString& CurrentBranch = FString()) const override { return false; }
	virtual bool IsCheckedOutOrModifiedInOtherBranch(const FString& CurrentBranch = FString()) const override { return false; }
	virtual TArray<FString> GetCheckedOutBranches() const override { return TArray<FString>(); }
	virtual FString GetOtherUserBranchCheckedOuts() const override { return FString(); }
	virtual bool GetOtherBranchHeadModification(FString& HeadBranchOut, FString& ActionOut, int32& HeadChangeListOut) const override { return false; }
	virtual bool IsCurrent() const override { return true; }
	virtual bool IsSourceControlled() const override;
	virtual bool IsAdded() const override;
	virtual bool IsDeleted() const override;
	virtual bool IsIgnored() const override { return false; }
	virtual bool CanEdit() const override;
	virtual bool CanDelete() const override;
	virtual bool IsUnknown() const override;
	virtual bool IsModified() const override;
	virtual bool CanAdd() const override;
	virtual bool IsConflicted() const override { return false; }
	virtual bool CanRevert() const override;

private:
	FString Filename;
	FDateTime TimeStamp;
	TArray<TSharedRef<class ISourceControlRevision, ESPMode::ThreadSafe>> History;
};
