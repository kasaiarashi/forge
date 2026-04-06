// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#include "ForgeSourceControlModule.h"
#include "ForgeSourceControlProvider.h"
#include "ISourceControlModule.h"
#include "Modules/ModuleManager.h"

#define LOCTEXT_NAMESPACE "ForgeSourceControl"

void FForgeSourceControlModule::StartupModule()
{
	Provider = new FForgeSourceControlProvider();
	ISourceControlModule::Get().RegisterProvider(
		FName("Forge"),
		LOCTEXT("ForgeProviderName", "Forge"),
		*Provider
	);
}

void FForgeSourceControlModule::ShutdownModule()
{
	ISourceControlModule::Get().UnregisterProvider(*Provider);
	delete Provider;
	Provider = nullptr;
}

#undef LOCTEXT_NAMESPACE

IMPLEMENT_MODULE(FForgeSourceControlModule, ForgeSourceControl)
