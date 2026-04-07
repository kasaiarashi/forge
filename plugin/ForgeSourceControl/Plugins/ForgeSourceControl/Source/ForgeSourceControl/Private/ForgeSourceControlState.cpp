// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#include "ForgeSourceControlState.h"
#include "Styling/AppStyle.h"

#define LOCTEXT_NAMESPACE "ForgeSourceControl"

TSharedPtr<ISourceControlRevision, ESPMode::ThreadSafe>
FForgeSourceControlState::GetHistoryItem(int32 HistoryIndex) const
{
	if (History.IsValidIndex(HistoryIndex))
	{
		return History[HistoryIndex];
	}
	return nullptr;
}

TSharedPtr<ISourceControlRevision, ESPMode::ThreadSafe>
FForgeSourceControlState::FindHistoryRevision(int32 RevisionNumber) const
{
	// TODO: implement revision lookup
	return nullptr;
}

TSharedPtr<ISourceControlRevision, ESPMode::ThreadSafe>
FForgeSourceControlState::FindHistoryRevision(const FString& InRevision) const
{
	// TODO: implement revision lookup by hash
	return nullptr;
}

FSlateIcon FForgeSourceControlState::GetIcon() const
{
	switch (FileState)
	{
	case EFileState::Modified:      return FSlateIcon(FAppStyle::GetAppStyleSetName(), "Perforce.CheckedOut");
	case EFileState::Added:         return FSlateIcon(FAppStyle::GetAppStyleSetName(), "Perforce.OpenForAdd");
	case EFileState::Deleted:       return FSlateIcon(FAppStyle::GetAppStyleSetName(), "Perforce.MarkedForDelete");
	case EFileState::Locked:        return FSlateIcon(FAppStyle::GetAppStyleSetName(), "Perforce.CheckedOut");
	case EFileState::LockedByOther: return FSlateIcon(FAppStyle::GetAppStyleSetName(), "Perforce.CheckedOutByOtherUser");
	case EFileState::Untracked:     return FSlateIcon(FAppStyle::GetAppStyleSetName(), "Perforce.NotInDepot");
	default:                        return FSlateIcon();
	}
}

FText FForgeSourceControlState::GetDisplayName() const
{
	switch (FileState)
	{
	case EFileState::Modified:      return LOCTEXT("Modified", "Modified");
	case EFileState::Added:         return LOCTEXT("Added", "Added");
	case EFileState::Deleted:       return LOCTEXT("Deleted", "Deleted");
	case EFileState::Locked:        return LOCTEXT("Locked", "Checked Out");
	case EFileState::LockedByOther: return FText::Format(LOCTEXT("LockedBy", "Checked Out by {0}"), FText::FromString(LockOwner));
	case EFileState::Untracked:     return LOCTEXT("Untracked", "Not Under Source Control");
	default:                        return LOCTEXT("Unmodified", "Up to Date");
	}
}

FText FForgeSourceControlState::GetDisplayTooltip() const
{
	return GetDisplayName();
}

bool FForgeSourceControlState::CanCheckIn() const
{
	return FileState == EFileState::Locked
		|| FileState == EFileState::Modified
		|| FileState == EFileState::Added;
}

bool FForgeSourceControlState::CanCheckout() const
{
	return FileState == EFileState::Unmodified;
}

bool FForgeSourceControlState::IsCheckedOut() const
{
	return FileState == EFileState::Locked;
}

bool FForgeSourceControlState::IsCheckedOutOther(FString* Who) const
{
	if (FileState == EFileState::LockedByOther)
	{
		if (Who) { *Who = LockOwner; }
		return true;
	}
	return false;
}

bool FForgeSourceControlState::IsSourceControlled() const
{
	return FileState != EFileState::Untracked;
}

bool FForgeSourceControlState::IsAdded() const
{
	return FileState == EFileState::Added;
}

bool FForgeSourceControlState::IsDeleted() const
{
	return FileState == EFileState::Deleted;
}

bool FForgeSourceControlState::CanEdit() const
{
	return FileState != EFileState::LockedByOther;
}

bool FForgeSourceControlState::CanDelete() const
{
	return IsSourceControlled() && FileState != EFileState::LockedByOther;
}

bool FForgeSourceControlState::IsUnknown() const
{
	return FileState == EFileState::Untracked;
}

bool FForgeSourceControlState::IsModified() const
{
	return FileState == EFileState::Modified;
}

bool FForgeSourceControlState::CanAdd() const
{
	return FileState == EFileState::Untracked;
}

bool FForgeSourceControlState::CanRevert() const
{
	return FileState == EFileState::Modified
		|| FileState == EFileState::Added
		|| FileState == EFileState::Deleted
		|| FileState == EFileState::Locked;
}

#undef LOCTEXT_NAMESPACE
