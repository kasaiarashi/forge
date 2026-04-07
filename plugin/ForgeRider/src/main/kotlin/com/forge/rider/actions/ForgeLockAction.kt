package com.forge.rider.actions

import com.forge.rider.ForgeVcs
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys
import com.intellij.openapi.ui.Messages

class ForgeLockAction : AnAction("Lock File", "Lock a file for exclusive editing", null) {

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val file = e.getData(CommonDataKeys.VIRTUAL_FILE) ?: return
        val vcs = ForgeVcs.getInstance(project)

        val relativePath = file.path.removePrefix(project.basePath ?: "").removePrefix("/")
        val result = vcs.cli.lock(relativePath)

        if (result.success) {
            Messages.showInfoMessage(project, "Locked: $relativePath", "Forge Lock")
        } else {
            Messages.showErrorDialog(project, "Failed to lock: ${result.stderr}", "Forge Lock")
        }
    }

    override fun update(e: AnActionEvent) {
        val file = e.getData(CommonDataKeys.VIRTUAL_FILE)
        e.presentation.isEnabled = file != null && !file.isDirectory
    }
}
