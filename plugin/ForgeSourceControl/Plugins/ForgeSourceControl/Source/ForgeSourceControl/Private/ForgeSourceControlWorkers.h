// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#pragma once

#include "CoreMinimal.h"
#include "ForgeSourceControlState.h"

class FForgeSourceControlCommand;
class FForgeSourceControlProvider;

typedef TSharedRef<class IForgeWorker, ESPMode::ThreadSafe> FForgeWorkerRef;
DECLARE_DELEGATE_RetVal(FForgeWorkerRef, FGetForgeWorker);

/** Base interface for all forge source control workers. */
class IForgeWorker
{
public:
	virtual ~IForgeWorker() = default;

	/** Worker name (matches operation name). */
	virtual FName GetName() const = 0;

	/** Execute the operation. Runs on a BACKGROUND THREAD — do not access UE objects. */
	virtual bool Execute(FForgeSourceControlCommand& InCommand) = 0;

	/** Apply results to provider state cache. Runs on the GAME THREAD. */
	virtual bool UpdateStates() = 0;

protected:
	/** Provider reference, captured from command during Execute(). Safe to use in UpdateStates(). */
	FForgeSourceControlProvider* ProviderRef = nullptr;
};

// ── Connect ─────────────────────────────────────────────────────────────────

class FForgeConnectWorker : public IForgeWorker
{
public:
	virtual FName GetName() const override { return "Connect"; }
	virtual bool Execute(FForgeSourceControlCommand& InCommand) override;
	virtual bool UpdateStates() override;
};

// ── UpdateStatus ────────────────────────────────────────────────────────────

class FForgeUpdateStatusWorker : public IForgeWorker
{
public:
	virtual FName GetName() const override { return "UpdateStatus"; }
	virtual bool Execute(FForgeSourceControlCommand& InCommand) override;
	virtual bool UpdateStates() override;

private:
	TMap<FString, FForgeSourceControlState::EFileState> States;
	TMap<FString, FString> LockOwners;
	FString WorkspaceRoot;
};

// ── CheckOut (Lock) ─────────────────────────────────────────────────────────

class FForgeCheckOutWorker : public IForgeWorker
{
public:
	virtual FName GetName() const override { return "CheckOut"; }
	virtual bool Execute(FForgeSourceControlCommand& InCommand) override;
	virtual bool UpdateStates() override;

private:
	TArray<FString> LockedFiles;
};

// ── CheckIn (Commit + Push) ─────────────────────────────────────────────────

class FForgeCheckInWorker : public IForgeWorker
{
public:
	virtual FName GetName() const override { return "CheckIn"; }
	virtual bool Execute(FForgeSourceControlCommand& InCommand) override;
	virtual bool UpdateStates() override;
};

// ── MarkForAdd ──────────────────────────────────────────────────────────────

class FForgeMarkForAddWorker : public IForgeWorker
{
public:
	virtual FName GetName() const override { return "MarkForAdd"; }
	virtual bool Execute(FForgeSourceControlCommand& InCommand) override;
	virtual bool UpdateStates() override;
};

// ── Revert ──────────────────────────────────────────────────────────────────

class FForgeRevertWorker : public IForgeWorker
{
public:
	virtual FName GetName() const override { return "Revert"; }
	virtual bool Execute(FForgeSourceControlCommand& InCommand) override;
	virtual bool UpdateStates() override;
};

// ── Delete ──────────────────────────────────────────────────────────────────

class FForgeDeleteWorker : public IForgeWorker
{
public:
	virtual FName GetName() const override { return "Delete"; }
	virtual bool Execute(FForgeSourceControlCommand& InCommand) override;
	virtual bool UpdateStates() override;
};

// ── Sync (Pull) ─────────────────────────────────────────────────────────────

class FForgeSyncWorker : public IForgeWorker
{
public:
	virtual FName GetName() const override { return "Sync"; }
	virtual bool Execute(FForgeSourceControlCommand& InCommand) override;
	virtual bool UpdateStates() override;
};

/** Helper to create worker instances via delegate. */
template <typename WorkerType>
FForgeWorkerRef CreateForgeWorker()
{
	return MakeShareable(new WorkerType());
}
