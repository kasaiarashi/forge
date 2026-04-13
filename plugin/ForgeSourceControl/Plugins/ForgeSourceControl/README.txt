Forge Source Control Plugin for Unreal Engine
==============================================

Overview
--------
Forge Source Control integrates Forge VCS into Unreal Engine's built-in source
control system. It provides native binary asset support and file locking,
designed specifically for game development teams.

Requirements
------------
- Unreal Engine 5.7
- Forge CLI (the `forge` command-line tool)

Installing the Forge CLI
------------------------
Download and install the Forge CLI from the official repository:

    https://github.com/kasaiarashi/forge/releases

After downloading, place the `forge` executable somewhere in your system PATH,
or configure the path in the plugin settings (see below).

Verify installation by running:

    forge --version

Plugin Installation
-------------------
1. Copy the "ForgeSourceControl" folder into your project's Plugins/ directory.
2. Restart the Unreal Editor.
3. Go to Edit > Project Settings > Source Control (or the Source Control toolbar
   dropdown) and select "Forge" as your provider.

Setup
-----
There are two ways to set up a Forge workspace:

  A) From the command line (recommended):
     1. Open a terminal in your project root directory.
     2. Run: forge init
     3. Run: forge remote add origin <your-server-url>
     4. Run: forge login --server <your-server-url>
     5. Restart the editor or re-select the Forge provider.

  B) From the editor:
     1. Select "Forge" as your source control provider.
     2. If no workspace is detected, a setup panel appears.
     3. Enter your remote URL (optional) and click "Initialize Project with
        Forge".
     4. If a remote URL is provided and you are not yet authenticated, an
        in-editor sign-in dialog opens so you can enter a username/password
        or personal access token without leaving the editor.

Usage
-----
Once connected, Forge integrates with Unreal Engine's source control workflow:

  - Check Out:   Right-click an asset > Source Control > Check Out
                  Acquires a file lock via `forge lock`.

  - Check In:    Right-click an asset > Source Control > Check In
                  Stages, commits, unlocks, and pushes the file.

  - Mark for Add: New assets are automatically staged via `forge add`.

  - Revert:      Right-click an asset > Source Control > Revert
                  Restores the file and releases any lock.

  - Delete:      Right-click an asset > Source Control > Delete
                  Removes the file via `forge rm`.

  - Status:      Asset icons in the Content Browser reflect the current state:
                  modified, added, deleted, locked, locked by another user,
                  or not under source control.

Configuration
-------------
In the Source Control settings panel you can configure:

  - Forge Executable Path: Path to the `forge` binary. Defaults to "forge"
    (uses PATH lookup). Set an absolute path if the CLI is not on your PATH.

Troubleshooting
---------------
  - "No Forge workspace found": The plugin walks up from the project directory
    looking for a .forge/ folder. Make sure you ran `forge init` in or above
    your project root.

  - Lock denied: Another user may already hold a lock on the file. Check who
    holds it via `forge lock list` or in the asset's Source Control tooltip.

  - Push failed: Ensure you have network connectivity and are authenticated
    with the remote server (`forge whoami --server <url>`).

Support and Documentation
-------------------------
  - Full documentation: https://github.com/kasaiarashi/forge#readme
  - Issue tracker:      https://github.com/kasaiarashi/forge/issues
  - Fab listing:        https://www.fab.com/listings/7cd90180-8c2f-4b64-a772-2f010cec0105

License
-------
MIT License. Copyright (c) 2026 Krishna Teja Mekala.
