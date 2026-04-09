// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#pragma once

#include "Modules/ModuleInterface.h"

class FForgeSourceControlProvider;

class FForgeSourceControlModule : public IModuleInterface
{
public:
	virtual void StartupModule() override;
	virtual void ShutdownModule() override;

	FForgeSourceControlModule() {}

	FForgeSourceControlProvider& GetProvider() { return *Provider; }

private:
	FForgeSourceControlProvider* Provider = nullptr;
};
