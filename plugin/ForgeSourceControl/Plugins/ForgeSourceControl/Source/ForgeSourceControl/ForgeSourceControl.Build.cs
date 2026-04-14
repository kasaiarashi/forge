// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

using UnrealBuildTool;

public class ForgeSourceControl : ModuleRules
{
	public ForgeSourceControl(ReadOnlyTargetRules Target) : base(Target)
	{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;

		PrivateDependencyModuleNames.AddRange(new string[]
		{
			"Core",
			"CoreUObject",
			"Slate",
			"SlateCore",
			"InputCore",
			"SourceControl",
			"Json",
			"JsonUtilities",
		});
	}
}
