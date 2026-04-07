// Live Coding Remote - HTTP endpoint for triggering Live Coding compilation.
// Drop into any UE5 project. No project-specific dependencies.

#include "LiveCodingRemoteModule.h"
#include "ILiveCodingModule.h"
#include "HttpServerModule.h"
#include "IHttpRouter.h"
#include "HttpServerResponse.h"
#include "Dom/JsonObject.h"
#include "Serialization/JsonWriter.h"
#include "Serialization/JsonSerializer.h"
#include "Misc/OutputDeviceRedirector.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#include "SocketSubsystem.h"
#include "Sockets.h"

DEFINE_LOG_CATEGORY_STATIC(LogLiveCodingRemote, Log, All);

/** Check if a TCP port is available by attempting a raw socket bind. */
static bool IsPortAvailable(uint32 Port)
{
	ISocketSubsystem* SocketSub = ISocketSubsystem::Get(PLATFORM_SOCKETSUBSYSTEM);
	if (!SocketSub) return false;

	FSocket* Sock = SocketSub->CreateSocket(NAME_Stream, TEXT("PortCheck"), false);
	if (!Sock) return false;

	Sock->SetReuseAddr(false);
	TSharedRef<FInternetAddr> Addr = SocketSub->CreateInternetAddr();
	Addr->SetIp(0x7F000001); // 127.0.0.1
	Addr->SetPort(Port);

	bool bAvailable = Sock->Bind(*Addr);
	Sock->Close();
	SocketSub->DestroySocket(Sock);

	return bAvailable;
}

/**
 * Extract compiler/linker error lines from the UnrealBuildTool Log.txt.
 * This plain-text log captures MSVC errors and Live++ patch failures.
 */
static TArray<FString> ExtractErrorsFromBuildLog()
{
	TArray<FString> Errors;

	FString BuildLogPath = FPaths::Combine(
		FPlatformProcess::UserSettingsDir(), TEXT("UnrealBuildTool"), TEXT("Log.txt"));

	FString Content;
	if (!FFileHelper::LoadFileToString(Content, *BuildLogPath))
	{
		return Errors;
	}

	TArray<FString> Lines;
	Content.ParseIntoArrayLines(Lines, true);

	for (const FString& Line : Lines)
	{
		FString Trimmed = Line.TrimStartAndEnd();
		if (Trimmed.Len() < 10)
		{
			continue;
		}

		// MSVC compiler/linker errors
		if (Trimmed.Contains(TEXT("): error C")) ||
			Trimmed.Contains(TEXT("): fatal error")) ||
			Trimmed.Contains(TEXT(": error LNK")) ||
			Trimmed.Contains(TEXT(": error C")))
		{
			Errors.Add(Trimmed);
		}
		// Live++ patch errors
		else if (Trimmed.Contains(TEXT("Failed to link patch")) ||
			Trimmed.Contains(TEXT("Failed to compile patch")) ||
			(Trimmed.Contains(TEXT("Patch creation")) && Trimmed.Contains(TEXT("failed"))))
		{
			Errors.Add(Trimmed);
		}
	}

	return Errors;
}

static FString CompileResultToString(ELiveCodingCompileResult Result)
{
	switch (Result)
	{
	case ELiveCodingCompileResult::Success:				return TEXT("Success");
	case ELiveCodingCompileResult::NoChanges:			return TEXT("NoChanges");
	case ELiveCodingCompileResult::InProgress:			return TEXT("InProgress");
	case ELiveCodingCompileResult::CompileStillActive:	return TEXT("CompileStillActive");
	case ELiveCodingCompileResult::NotStarted:			return TEXT("NotStarted");
	case ELiveCodingCompileResult::Failure:				return TEXT("Failure");
	case ELiveCodingCompileResult::Cancelled:			return TEXT("Cancelled");
	default:											return TEXT("Unknown");
	}
}

void FLiveCodingRemoteModule::StartupModule()
{
	// Find an available port using raw socket check BEFORE calling GetHttpRouter.
	// GetHttpRouter triggers the engine's internal bind which logs errors on failure
	// and can cause cook processes to abort.
	const uint32 MaxAttempts = 10;
	uint32 ChosenPort = 0;

	for (uint32 i = 0; i < MaxAttempts; ++i)
	{
		uint32 TryPort = ServerPort + i;
		if (IsPortAvailable(TryPort))
		{
			ChosenPort = TryPort;
			break;
		}
		UE_LOG(LogLiveCodingRemote, Display, TEXT("Port %d in use, trying %d..."), TryPort, TryPort + 1);
	}

	if (ChosenPort == 0)
	{
		UE_LOG(LogLiveCodingRemote, Warning, TEXT("No available port in range %d-%d, skipping HTTP server"),
			ServerPort, ServerPort + MaxAttempts - 1);
		return;
	}

	ServerPort = ChosenPort;

	FHttpServerModule& HttpServer = FHttpServerModule::Get();
	TSharedPtr<IHttpRouter> Router = HttpServer.GetHttpRouter(ServerPort);

	if (!Router)
	{
		UE_LOG(LogLiveCodingRemote, Warning, TEXT("Failed to get HTTP router on port %d"), ServerPort);
		return;
	}

	CompileRouteHandle = Router->BindRoute(
		FHttpPath(TEXT("/compile")),
		EHttpServerRequestVerbs::VERB_POST | EHttpServerRequestVerbs::VERB_GET,
		FHttpRequestHandler::CreateRaw(this, &FLiveCodingRemoteModule::HandleCompileRequest)
	);

	HttpServer.StartAllListeners();

	UE_LOG(LogLiveCodingRemote, Display, TEXT("Live Coding Remote server started on port %d"), ServerPort);
}

void FLiveCodingRemoteModule::ShutdownModule()
{
	if (CompileRouteHandle)
	{
		FHttpServerModule& HttpServer = FHttpServerModule::Get();
		if (TSharedPtr<IHttpRouter> Router = HttpServer.GetHttpRouter(ServerPort))
		{
			Router->UnbindRoute(CompileRouteHandle);
		}
	}
}

bool FLiveCodingRemoteModule::HandleCompileRequest(
	const FHttpServerRequest& Request,
	const FHttpResultCallback& OnComplete)
{
	// Build response JSON
	TSharedRef<FJsonObject> ResponseJson = MakeShared<FJsonObject>();

	ILiveCodingModule* LiveCoding = FModuleManager::GetModulePtr<ILiveCodingModule>(LIVE_CODING_MODULE_NAME);
	if (!LiveCoding)
	{
		ResponseJson->SetBoolField(TEXT("success"), false);
		ResponseJson->SetStringField(TEXT("status"), TEXT("NotAvailable"));
		TArray<TSharedPtr<FJsonValue>> Logs;
		Logs.Add(MakeShared<FJsonValueString>(TEXT("Live Coding module not loaded")));
		ResponseJson->SetArrayField(TEXT("logs"), Logs);

		FString Body;
		TSharedRef<TJsonWriter<>> Writer = TJsonWriterFactory<>::Create(&Body);
		FJsonSerializer::Serialize(ResponseJson, Writer);

		auto Response = FHttpServerResponse::Create(Body, TEXT("application/json"));
		OnComplete(MoveTemp(Response));
		return true;
	}

	if (!LiveCoding->IsEnabledForSession())
	{
		ResponseJson->SetBoolField(TEXT("success"), false);
		ResponseJson->SetStringField(TEXT("status"), TEXT("NotEnabled"));
		TArray<TSharedPtr<FJsonValue>> Logs;
		Logs.Add(MakeShared<FJsonValueString>(TEXT("Live Coding not enabled for this session")));
		ResponseJson->SetArrayField(TEXT("logs"), Logs);

		FString Body;
		TSharedRef<TJsonWriter<>> Writer = TJsonWriterFactory<>::Create(&Body);
		FJsonSerializer::Serialize(ResponseJson, Writer);

		auto Response = FHttpServerResponse::Create(Body, TEXT("application/json"));
		OnComplete(MoveTemp(Response));
		return true;
	}

	// Delete old build log so we only see errors from this compilation
	FString BuildLogPath = FPaths::Combine(
		FPlatformProcess::UserSettingsDir(), TEXT("UnrealBuildTool"), TEXT("Log.txt"));
	IFileManager::Get().Delete(*BuildLogPath, false, false, true);

	// Trigger synchronous compilation
	ELiveCodingCompileResult Result = ELiveCodingCompileResult::Failure;
	LiveCoding->Compile(ELiveCodingCompileFlags::WaitForCompletion, &Result);

	// Always check log for errors — Live++ may report Success even when patches fail
	TArray<FString> Errors = ExtractErrorsFromBuildLog();

	bool bIsSuccess = (Result == ELiveCodingCompileResult::Success || Result == ELiveCodingCompileResult::NoChanges);
	if (bIsSuccess && Errors.Num() > 0)
	{
		bIsSuccess = false;
	}
	ResponseJson->SetBoolField(TEXT("success"), bIsSuccess);
	ResponseJson->SetStringField(TEXT("status"), bIsSuccess ? CompileResultToString(Result) : TEXT("Failure"));

	TArray<TSharedPtr<FJsonValue>> LogArray;
	for (const FString& Line : Errors)
	{
		LogArray.Add(MakeShared<FJsonValueString>(Line));
	}
	ResponseJson->SetArrayField(TEXT("logs"), LogArray);

	FString Body;
	TSharedRef<TJsonWriter<>> Writer = TJsonWriterFactory<>::Create(&Body);
	FJsonSerializer::Serialize(ResponseJson, Writer);

	UE_LOG(LogLiveCodingRemote, Display, TEXT("Compile result: %s"), *CompileResultToString(Result));

	auto Response = FHttpServerResponse::Create(Body, TEXT("application/json"));
	OnComplete(MoveTemp(Response));
	return true;
}

IMPLEMENT_MODULE(FLiveCodingRemoteModule, LiveCodingRemote)
