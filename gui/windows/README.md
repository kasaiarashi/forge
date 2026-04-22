# Forge GUI — Windows

WinUI 3 desktop client for Forge VCS. Part of the Forge monorepo; a future `gui/mac/` will live beside this.

## Status

**P0 — shell + mock data.** No FFI wiring yet; the app talks to an in-memory `MockForgeClient` with 3 fake repos. This milestone exists to pin the visual system and page layout before real data churn.

## Stack

- WinUI 3 + .NET 8 (x64, Windows 10 1809+, Windows 11)
- CommunityToolkit.Mvvm for observable VMs
- MSIX packaging via WindowsAppSDK
- FFI via `forge_ffi.dll` (P/Invoke, wired in P1)

## Projects

- `Forge.Gui.App` — WinUI shell, XAML pages, MSIX manifest
- `Forge.Gui.Core` — VMs, models, services (no XAML)
- `Forge.Gui.Ffi` — P/Invoke bindings (skeleton; P1)
- `Forge.Gui.Mock` — in-memory `IForgeClient`
- `Forge.Gui.Tests` — xUnit

## Build

Requires:

- Visual Studio 2022 17.9+ with **Windows App SDK C# Templates** workload, OR `dotnet` SDK 8 + Windows App SDK runtime
- Windows 10 SDK 10.0.22621 or newer

```powershell
cd gui/windows
dotnet restore
dotnet build -c Debug -p:Platform=x64
```

Launch:

```powershell
dotnet run --project src/Forge.Gui.App -c Debug -p:Platform=x64
```

Or open `Forge.Gui.sln` in Visual Studio and hit F5.

## Test

```powershell
dotnet test
```

## Roadmap

See `../../.claude/plans/i-want-to-make-rosy-cosmos.md` (plan doc) for the phase-by-phase rollout. Current phase: **P0**. Next: **P1** — wire `Forge.Gui.Ffi` to `forge_ffi.dll`, swap DI registration away from mock.
