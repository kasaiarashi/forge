using Forge.Gui.Core.Models;

namespace Forge.Gui.Mock;

internal static class Fixtures
{
    public static WorkspaceInfo Workspace => new(
        Path: @"W:\Projects\DemoArena",
        RepoName: "DemoArena",
        ServerUrl: "https://forge.local:50051",
        CurrentBranch: "dev",
        DefaultRemote: "origin");

    public static IReadOnlyList<FileChange> Changes =>
    [
        new("Content/Characters/Hero.uasset",              FileChangeKind.StagedModified, true,  "krishna"),
        new("Content/Maps/Arena.umap",                     FileChangeKind.StagedNew,      false, null),
        new("Content/UI/MainMenu.uasset",                  FileChangeKind.Modified,       false, null),
        new("Source/DemoArena/Private/HeroChar.cpp",       FileChangeKind.Modified,       false, null),
        new("Source/DemoArena/Private/HeroChar.h",         FileChangeKind.StagedModified, false, null),
        new("Config/DefaultEngine.ini",                    FileChangeKind.Modified,       false, null),
        new("Content/FX/Explosion.uasset",                 FileChangeKind.Untracked,      false, null),
        new("Docs/old_design.md",                          FileChangeKind.StagedDeleted,  false, null),
        new("Content/Audio/theme.uasset",                  FileChangeKind.Modified,       true,  "riley"),
    ];

    public static IReadOnlyList<LogEntry> Log =>
    [
        new("a1b2c3d4", ["e5f6a7b8"],                     "Krishna Teja", "krishna@kriaa.in",
            DateTimeOffset.Now.AddHours(-2),  "combat: hero dash polish"),
        new("e5f6a7b8", ["c9d0e1f2", "3344aabb"],         "Krishna Teja", "krishna@kriaa.in",
            DateTimeOffset.Now.AddHours(-6),  "merge feature/arena into dev"),
        new("c9d0e1f2", ["9a8b7c6d"],                     "Riley Chen",   "riley@studio.test",
            DateTimeOffset.Now.AddDays(-1),   "audio: swap theme stem, retime intro"),
        new("3344aabb", ["9a8b7c6d"],                     "Morgan Park",  "morgan@studio.test",
            DateTimeOffset.Now.AddDays(-1),   "arena: new cover props + lighting pass"),
        new("9a8b7c6d", ["11112222"],                     "Krishna Teja", "krishna@kriaa.in",
            DateTimeOffset.Now.AddDays(-3),   "release: 0.2.5 — bypass-forgeignore flag"),
        new("11112222", ["33334444"],                     "Riley Chen",   "riley@studio.test",
            DateTimeOffset.Now.AddDays(-4),   "ui: main menu nav overhaul"),
        new("33334444", [],                               "Krishna Teja", "krishna@kriaa.in",
            DateTimeOffset.Now.AddDays(-30),  "initial commit"),
    ];

    public static IReadOnlyList<Branch> Branches =>
    [
        new("dev",              IsCurrent: true,  IsRemote: false, TipHash: "a1b2c3d4", AheadOfUpstream: 2, BehindUpstream: 0),
        new("main",             IsCurrent: false, IsRemote: false, TipHash: "9a8b7c6d", AheadOfUpstream: 0, BehindUpstream: 0),
        new("feature/arena",    IsCurrent: false, IsRemote: false, TipHash: "3344aabb", AheadOfUpstream: 0, BehindUpstream: 1),
        new("origin/dev",       IsCurrent: false, IsRemote: true,  TipHash: "e5f6a7b8", AheadOfUpstream: null, BehindUpstream: null),
        new("origin/main",      IsCurrent: false, IsRemote: true,  TipHash: "9a8b7c6d", AheadOfUpstream: null, BehindUpstream: null),
    ];

    public static IReadOnlyList<LockInfo> Locks =>
    [
        new("Content/Characters/Hero.uasset", "krishna", DateTimeOffset.Now.AddMinutes(-12), "dash anim pass"),
        new("Content/Audio/theme.uasset",     "riley",   DateTimeOffset.Now.AddHours(-3),    "mixing stems"),
        new("Content/Maps/Arena.umap",        "morgan",  DateTimeOffset.Now.AddHours(-8),    "lighting bake"),
    ];
}
