// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#pragma once

#include "ISourceControlOperation.h"
#include "ISourceControlProvider.h"
#include "Misc/IQueuedWork.h"
#include "HAL/PlatformAtomics.h"

class IForgeWorker;
class FForgeSourceControlProvider;

/**
 * Command wrapper that implements IQueuedWork for thread pool dispatch.
 * Created by Execute(), queued to GThreadPool, processed by Tick().
 */
class FForgeSourceControlCommand : public IQueuedWork
{
public:
	FForgeSourceControlCommand(
		const FSourceControlOperationRef& InOperation,
		const TSharedRef<IForgeWorker, ESPMode::ThreadSafe>& InWorker,
		EConcurrency::Type InConcurrency,
		FForgeSourceControlProvider& InProvider)
		: Operation(InOperation)
		, Worker(InWorker)
		, Concurrency(InConcurrency)
		, Provider(InProvider)
		, bExecuteProcessed(0)
		, bCommandSuccessful(false)
		, bAutoDelete(true)
	{
	}

	// IQueuedWork interface
	virtual void DoThreadedWork() override;
	virtual void Abandon() override;

	/** Signal that the operation is complete (called from worker thread). */
	void MarkOperationCompleted(bool bSuccess);

	/** Transfer messages to Operation and fire the completion delegate (called from game thread). */
	ECommandResult::Type ReturnResults();

	// Operation and worker
	FSourceControlOperationRef Operation;
	TSharedRef<IForgeWorker, ESPMode::ThreadSafe> Worker;
	FSourceControlOperationComplete OperationCompleteDelegate;
	EConcurrency::Type Concurrency;

	// Provider reference — workers use this instead of FModuleManager (thread-safe).
	FForgeSourceControlProvider& Provider;

	// Files to operate on (absolute paths)
	TArray<FString> Files;

	// Result messages
	TArray<FString> InfoMessages;
	TArray<FString> ErrorMessages;

	// Atomic completion flag — set by worker thread, read by game thread in Tick()
	volatile int32 bExecuteProcessed;

	// Whether the command succeeded
	bool bCommandSuccessful;

	// If true, Tick() deletes the command after processing (async commands).
	// If false, ExecuteSynchronousCommand deletes it (sync commands).
	bool bAutoDelete;
};
