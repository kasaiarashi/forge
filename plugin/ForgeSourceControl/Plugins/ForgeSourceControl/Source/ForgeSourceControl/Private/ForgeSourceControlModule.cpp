// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

#include "ForgeSourceControlModule.h"
#include "ForgeFFIBridge.h"
#include "ForgeSourceControlProvider.h"
#include "ForgeSourceControlWorkers.h"
#include "Features/IModularFeatures.h"
#include "Modules/ModuleManager.h"

#define LOCTEXT_NAMESPACE "ForgeSourceControl"

void FForgeSourceControlModule::StartupModule()
{
	// Phase 4c.1 — load the Rust FFI library ONCE per editor session.
	// Workers will prefer it over the CLI subprocess path when
	// available; if it fails to load, everything keeps working via
	// the legacy code.
	FForgeFFI::Initialize();

	Provider = new FForgeSourceControlProvider();

	// Register workers for each supported operation.
	Provider->RegisterWorker("Connect",
		FGetForgeWorker::CreateStatic(&CreateForgeWorker<FForgeConnectWorker>));
	Provider->RegisterWorker("UpdateStatus",
		FGetForgeWorker::CreateStatic(&CreateForgeWorker<FForgeUpdateStatusWorker>));
	Provider->RegisterWorker("CheckOut",
		FGetForgeWorker::CreateStatic(&CreateForgeWorker<FForgeCheckOutWorker>));
	Provider->RegisterWorker("CheckIn",
		FGetForgeWorker::CreateStatic(&CreateForgeWorker<FForgeCheckInWorker>));
	Provider->RegisterWorker("MarkForAdd",
		FGetForgeWorker::CreateStatic(&CreateForgeWorker<FForgeMarkForAddWorker>));
	Provider->RegisterWorker("Revert",
		FGetForgeWorker::CreateStatic(&CreateForgeWorker<FForgeRevertWorker>));
	Provider->RegisterWorker("Delete",
		FGetForgeWorker::CreateStatic(&CreateForgeWorker<FForgeDeleteWorker>));
	Provider->RegisterWorker("Sync",
		FGetForgeWorker::CreateStatic(&CreateForgeWorker<FForgeSyncWorker>));

	IModularFeatures::Get().RegisterModularFeature("SourceControl", Provider);
}

void FForgeSourceControlModule::ShutdownModule()
{
	IModularFeatures::Get().UnregisterModularFeature("SourceControl", Provider);
	delete Provider;
	Provider = nullptr;

	FForgeFFI::Shutdown();
}

#undef LOCTEXT_NAMESPACE

IMPLEMENT_MODULE(FForgeSourceControlModule, ForgeSourceControl)
