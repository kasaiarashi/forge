using UnrealBuildTool;

public class LiveCodingRemote : ModuleRules
{
	public LiveCodingRemote(ReadOnlyTargetRules Target) : base(Target)
	{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;

		PublicDependencyModuleNames.AddRange(new string[]
		{
			"Core"
		});

		PrivateDependencyModuleNames.AddRange(new string[]
		{
			"CoreUObject",
			"Engine",
			"HTTPServer",
			"Json",
			"JsonUtilities",
			"LiveCoding",
			"Sockets",
			"Networking"
		});
	}
}
