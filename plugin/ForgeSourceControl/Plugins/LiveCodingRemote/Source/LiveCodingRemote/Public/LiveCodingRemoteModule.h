// Live Coding Remote - HTTP endpoint for triggering Live Coding compilation.
// Drop into any UE5 project. No project-specific dependencies.

#pragma once

#include "CoreMinimal.h"
#include "Modules/ModuleManager.h"
#include "HttpServerModule.h"
#include "IHttpRouter.h"

class FLiveCodingRemoteModule : public IModuleInterface
{
public:
	virtual void StartupModule() override;
	virtual void ShutdownModule() override;

private:
	/** Handle POST /compile requests */
	bool HandleCompileRequest(const FHttpServerRequest& Request, const FHttpResultCallback& OnComplete);

	/** HTTP router handle for cleanup */
	FHttpRouteHandle CompileRouteHandle;

	/** Port for the HTTP server */
	uint32 ServerPort = 1800;
};
