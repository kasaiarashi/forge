package com.forge.rider

import com.intellij.openapi.project.Project
import com.intellij.openapi.vcs.FilePath
import com.intellij.openapi.vcs.VcsException
import com.intellij.openapi.vcs.changes.Change
import com.intellij.openapi.vcs.rollback.RollbackEnvironment
import com.intellij.openapi.vcs.rollback.RollbackProgressListener
import com.intellij.openapi.vfs.VirtualFile

class ForgeRollbackEnvironment(
    private val project: Project,
    private val cli: ForgeCliRunner,
) : RollbackEnvironment {

    override fun getRollbackOperationName(): String = "Revert"

    override fun rollbackChanges(
        changes: MutableList<out Change>,
        exceptions: MutableList<VcsException>,
        listener: RollbackProgressListener,
    ) {
        for (change in changes) {
            val path = (change.afterRevision?.file ?: change.beforeRevision?.file) ?: continue
            val relativePath = toRelative(path.path)
            listener.accept(change)
            cli.unstage(relativePath)
            cli.restore(relativePath)
        }
    }

    override fun rollbackMissingFileDeletion(
        files: MutableList<out FilePath>,
        exceptions: MutableList<in VcsException>,
        listener: RollbackProgressListener,
    ) {
        for (file in files) {
            cli.restore(toRelative(file.path))
        }
    }

    override fun rollbackModifiedWithoutCheckout(
        files: MutableList<out VirtualFile>,
        exceptions: MutableList<in VcsException>,
        listener: RollbackProgressListener,
    ) {
        for (file in files) {
            cli.restore(toRelative(file.path))
        }
    }

    private fun toRelative(absolutePath: String): String {
        val basePath = project.basePath ?: return absolutePath
        return absolutePath.removePrefix(basePath).removePrefix("/").removePrefix("\\")
    }
}
