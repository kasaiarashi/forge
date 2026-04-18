// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

using System.IO;
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
			"Projects",       // Phase 4c.1 — IPluginManager for DLL path resolution.
			"Slate",
			"SlateCore",
			"InputCore",
			"SourceControl",
			"Json",
			"JsonUtilities",
		});

		// Phase 4c.1 — include path for the Rust FFI header. Relative
		// to this .Build.cs so developers cloning the repo at any
		// checkout depth pick it up without extra configuration.
		//
		// Computed as: <repo>/crates/forge-ffi/include
		// From this file at: <repo>/plugin/ForgeSourceControl/Plugins/ForgeSourceControl/Source/ForgeSourceControl/
		// → six `..` segments up, then down into crates/forge-ffi/include.
		string RepoRoot = Path.GetFullPath(Path.Combine(ModuleDirectory,
			"..", "..", "..", "..", "..", ".."));
		string ForgeFfiIncludeDir = Path.Combine(RepoRoot, "crates", "forge-ffi", "include");
		if (Directory.Exists(ForgeFfiIncludeDir))
		{
			PrivateIncludePaths.Add(ForgeFfiIncludeDir);
		}
	}
}
