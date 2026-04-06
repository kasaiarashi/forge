// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#pragma once

#include "Modules/ModuleInterface.h"

class FForgeSourceControlModule : public IModuleInterface
{
public:
	virtual void StartupModule() override;
	virtual void ShutdownModule() override;

	FForgeSourceControlModule() {}

private:
	/** The provider instance registered with the source control framework. */
	class FForgeSourceControlProvider* Provider = nullptr;
};
