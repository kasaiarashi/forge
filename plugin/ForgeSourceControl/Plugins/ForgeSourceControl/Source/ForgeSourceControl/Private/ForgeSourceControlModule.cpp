// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#include "ForgeSourceControlModule.h"
#include "ForgeSourceControlProvider.h"
#include "Features/IModularFeatures.h"
#include "Modules/ModuleManager.h"

#define LOCTEXT_NAMESPACE "ForgeSourceControl"

void FForgeSourceControlModule::StartupModule()
{
	Provider = new FForgeSourceControlProvider();
	IModularFeatures::Get().RegisterModularFeature("SourceControl", Provider);
}

void FForgeSourceControlModule::ShutdownModule()
{
	IModularFeatures::Get().UnregisterModularFeature("SourceControl", Provider);
	delete Provider;
	Provider = nullptr;
}

#undef LOCTEXT_NAMESPACE

IMPLEMENT_MODULE(FForgeSourceControlModule, ForgeSourceControl)
