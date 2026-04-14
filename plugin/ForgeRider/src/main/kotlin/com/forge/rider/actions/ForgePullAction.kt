package com.forge.rider.actions

import com.forge.rider.ForgeVcs
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.progress.ProgressIndicator
import com.intellij.openapi.progress.ProgressManager
import com.intellij.openapi.progress.Task
import com.intellij.openapi.ui.Messages

class ForgePullAction : AnAction("Pull", "Pull changes from remote", null) {

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val vcs = ForgeVcs.getInstance(project)

        ProgressManager.getInstance().run(object : Task.Backgroundable(project, "Pulling from remote...", true) {
            override fun run(indicator: ProgressIndicator) {
                val result = vcs.cli.pull()
                if (!result.success) {
                    Messages.showErrorDialog(project, "Pull failed: ${result.stderr}", "Forge Pull")
                }
            }
        })
    }
}
