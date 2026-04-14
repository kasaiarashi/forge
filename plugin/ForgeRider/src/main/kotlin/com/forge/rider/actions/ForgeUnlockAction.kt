package com.forge.rider.actions

import com.forge.rider.ForgeVcs
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys
import com.intellij.openapi.ui.Messages

class ForgeUnlockAction : AnAction("Unlock File", "Release file lock", null) {

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val file = e.getData(CommonDataKeys.VIRTUAL_FILE) ?: return
        val vcs = ForgeVcs.getInstance(project)

        val relativePath = file.path.removePrefix(project.basePath ?: "").removePrefix("/")
        val result = vcs.cli.unlock(relativePath)

        if (result.success) {
            Messages.showInfoMessage(project, "Unlocked: $relativePath", "Forge Unlock")
        } else {
            Messages.showErrorDialog(project, "Failed to unlock: ${result.stderr}", "Forge Unlock")
        }
    }

    override fun update(e: AnActionEvent) {
        val file = e.getData(CommonDataKeys.VIRTUAL_FILE)
        e.presentation.isEnabled = file != null && !file.isDirectory
    }
}
