# Forge Source Control — User Guide

This guide walks you through installing, configuring, and using the Forge
Source Control plugin for Unreal Engine.

- Plugin version: 0.1.0
- Engine version: 5.7
- Platforms: Windows, macOS, Linux
- Online documentation: https://github.com/kasaiarashi/forge#readme
- Issue tracker: https://github.com/kasaiarashi/forge/issues

---

## 1. What is Forge?

Forge is a binary-first version control system built for game development. It
stores large binary assets efficiently (content-defined chunking, zstd
compression, BLAKE3 hashing) and supports file locking so only one person edits
a given asset at a time.

This plugin registers Forge as a source control provider inside Unreal Engine,
so you can check files in/out, commit, and lock assets straight from the
Content Browser — the same way you would with Perforce or Git.

---

## 2. Prerequisites

The plugin is a thin bridge to the Forge command-line tool. You need the CLI
installed and reachable before the plugin can do anything.

### 2.1 Install the Forge CLI

Download a prebuilt binary from the Forge releases page:

    https://github.com/kasaiarashi/forge/releases

After downloading, either:

- Place `forge` (or `forge.exe` on Windows) in a folder that is on your
  system `PATH`, or
- Note the absolute path — you will enter it in the plugin settings.

Verify the install:

    forge --version

Expected output: `forge <version>`.

### 2.2 (Optional) A Forge server

You only need a Forge server if you want to push to a remote, pull from
teammates, or use file locking. A local-only workspace works without one.

Server setup is documented at
https://github.com/kasaiarashi/forge#server — typically a colleague or studio
ops team runs it, and gives you a URL and an account.

---

## 3. Install the plugin

1. Close the Unreal Editor.
2. Copy the `ForgeSourceControl` folder into your project's `Plugins/`
   directory, so the final layout is:
   `<YourProject>/Plugins/ForgeSourceControl/ForgeSourceControl.uplugin`.
3. Launch the editor. The plugin is enabled by default.
4. Open **Edit → Project Settings → Plugins → Forge Source Control** to
   confirm it is listed and enabled.

---

## 4. First-time setup inside the editor

1. Click the source control dropdown in the toolbar and choose
   **Connect to Source Control…**.
2. Select **Forge** from the **Provider** dropdown.
3. The Forge settings panel appears.

### 4.1 Point the plugin at the `forge` executable

In the **Forge Executable Path** field:

- Leave the default value (`forge`) if the CLI is on your `PATH`.
- Otherwise enter an absolute path, for example:
  - Windows: `C:\Tools\forge\forge.exe`
  - macOS / Linux: `/usr/local/bin/forge`

### 4.2 Initialize a workspace

If the project has no `.forge/` directory yet, the panel shows an
**Initialize Project with Forge** section.

- **Remote URL** (optional): the Forge server URL, e.g.
  `https://forge.example.com/<owner>/<repo>`. Leave blank for a local-only
  workspace.
- Click **Initialize Project with Forge**. This runs `forge init` in the
  project directory, and — if a remote URL was given — adds it as `origin`.
- If you entered a remote URL and you are not yet signed in to that server,
  the **Forge Sign In** dialog opens. Enter either:
  - your username and password, or
  - a personal access token (PAT) — toggle **Use personal access token**
    and paste it in.
  Click **Sign In**. The plugin runs `forge login` for you and saves the
  credential in the platform credential store.

On success, the settings panel collapses the init section and the provider
reports as **Enabled / Available**.

---

## 5. Daily workflow

With the provider enabled, the standard Unreal source control menu applies to
every asset in the Content Browser.

| Action              | What the plugin does                                                |
| ------------------- | ------------------------------------------------------------------- |
| **Check Out**       | Acquires an exclusive file lock via `forge lock`.                   |
| **Mark for Add**    | Stages a new asset via `forge add`.                                 |
| **Check In**        | Stages, commits, releases the lock, and pushes the file.            |
| **Revert**          | Restores the working copy and releases any lock.                    |
| **Delete**          | Removes the file via `forge rm`.                                    |
| **Refresh**         | Re-reads status from the workspace and updates the Content Browser. |

Asset icons reflect the current state:

- Modified, added, or deleted locally
- Locked by you
- Locked by another user (read-only)
- Not under source control

Hovering an asset shows the lock owner, if any.

---

## 6. Configuration reference

All settings live in the **Source Control** settings panel:

- **Forge Executable Path** — path to `forge` (or `forge.exe`). Default `forge`.

Workspace-level config (identity, remotes) is stored by the CLI in
`<project>/.forge/config.json`. The plugin never writes to it directly — all
workspace changes go through the CLI.

---

## 7. Troubleshooting

### "No Forge workspace found"

The plugin walks up from the project directory looking for a `.forge/`
folder. Make sure `forge init` was run in the project root (or a parent
folder). You can also click **Initialize Project with Forge** from the
settings panel.

### "forge: command not found" / "Failed to run forge"

The CLI is not on `PATH`, or the **Forge Executable Path** is wrong. Run
`forge --version` in a terminal. If that works, point the plugin at that
same path. If it does not, re-install the CLI (see section 2).

### Check-out denied / "locked by <other user>"

Another user holds the lock. Ask them to release it, or use
`forge lock list` to see all locks on the repo.

### Push fails with "non-fast-forward"

The remote has commits you don't have locally. Pull first:

    forge pull

Then re-try the check-in. This guards against accidentally rewinding a
shared branch.

### Login dialog rejects credentials

- Confirm the remote URL is correct and reachable (open it in a browser).
- For username/password: the account must exist on that Forge server.
- For PAT: generate a new token from the server's web UI and paste it in.
- Run `forge whoami --server <url>` in a terminal for a detailed error.

---

## 8. Uninstall

1. Close the editor.
2. Delete `<YourProject>/Plugins/ForgeSourceControl/`.
3. (Optional) Delete the workspace's `.forge/` folder to remove all Forge
   tracking metadata. Your assets are untouched.

---

## 9. Support and feedback

- GitHub issues: https://github.com/kasaiarashi/forge/issues
- Fab listing: https://www.fab.com/listings/7cd90180-8c2f-4b64-a772-2f010cec0105

Licensed under the BSL 1.1.. Copyright (c) 2026 Krishna Teja Mekala.
