using UnrealBuildTool;
using System.Collections.Generic;

public class ForgeVCSEditorTarget : TargetRules
{
	public ForgeVCSEditorTarget(TargetInfo Target) : base(Target)
	{
		Type = TargetType.Editor;
		DefaultBuildSettings = BuildSettingsVersion.V6;

		ExtraModuleNames.AddRange( new string[] { "ForgeVCS" } );
	}
}
