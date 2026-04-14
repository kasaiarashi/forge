using UnrealBuildTool;
using System.Collections.Generic;

public class ForgeVCSTarget : TargetRules
{
	public ForgeVCSTarget(TargetInfo Target) : base(Target)
	{
		Type = TargetType.Game;
		DefaultBuildSettings = BuildSettingsVersion.V6;

		ExtraModuleNames.AddRange( new string[] { "ForgeVCS" } );
	}
}
