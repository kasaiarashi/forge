package com.forge.rider.actions

import com.forge.rider.ForgeVcs
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.ui.Messages

class ForgeStashPopAction : AnAction("Pop Stash", "Apply and remove the latest stash", null) {

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val vcs = ForgeVcs.getInstance(project)

        val result = vcs.cli.stashPop()

        if (result.success) {
            Messages.showInfoMessage(project, "Stash applied successfully.", "Forge Stash Pop")
        } else {
            Messages.showErrorDialog(project, "Stash pop failed: ${result.stderr}", "Forge Stash Pop")
        }
    }
}
