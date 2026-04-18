// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

#include "ForgeSourceControlCommand.h"
#include "ForgeSourceControlWorkers.h"

void FForgeSourceControlCommand::DoThreadedWork()
{
	Worker->Execute(*this);
}

void FForgeSourceControlCommand::Abandon()
{
	FPlatformAtomics::InterlockedExchange(&bExecuteProcessed, 1);
}

void FForgeSourceControlCommand::MarkOperationCompleted(bool bSuccess)
{
	bCommandSuccessful = bSuccess;
	FPlatformAtomics::InterlockedExchange(&bExecuteProcessed, 1);
}

ECommandResult::Type FForgeSourceControlCommand::ReturnResults()
{
	for (const FString& Msg : InfoMessages)
	{
		Operation->AddInfoMessge(FText::FromString(Msg));
	}
	for (const FString& Msg : ErrorMessages)
	{
		Operation->AddErrorMessge(FText::FromString(Msg));
	}

	ECommandResult::Type Result = bCommandSuccessful ? ECommandResult::Succeeded : ECommandResult::Failed;
	OperationCompleteDelegate.ExecuteIfBound(Operation, Result);
	return Result;
}
