package com.forge.rider.actions

import com.forge.rider.ForgeVcs
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.ui.Messages

class ForgeStashAction : AnAction("Stash Changes", "Stash current changes", null) {

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val vcs = ForgeVcs.getInstance(project)

        val message = Messages.showInputDialog(
            project,
            "Stash message (optional):",
            "Forge Stash",
            null,
        )

        val result = if (message.isNullOrBlank()) vcs.cli.stash() else vcs.cli.stash(message)

        if (result.success) {
            Messages.showInfoMessage(project, "Changes stashed successfully.", "Forge Stash")
        } else {
            Messages.showErrorDialog(project, "Stash failed: ${result.stderr}", "Forge Stash")
        }
    }
}
