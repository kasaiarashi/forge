package com.forge.rider

import com.intellij.openapi.project.Project
import com.intellij.openapi.vcs.FilePath
import com.intellij.openapi.vcs.VcsException
import com.intellij.openapi.vcs.changes.Change
import com.intellij.openapi.vcs.checkin.CheckinEnvironment
import com.intellij.openapi.vfs.VirtualFile

class ForgeCheckinEnvironment(
    private val project: Project,
    private val cli: ForgeCliRunner,
) : CheckinEnvironment {

    override fun getCheckinOperationName(): String = "Commit"

    override fun commit(
        changes: MutableList<out Change>,
        commitMessage: String,
    ): List<VcsException> {
        val errors = mutableListOf<VcsException>()

        val paths = changes.mapNotNull { change ->
            (change.afterRevision?.file ?: change.beforeRevision?.file)?.path
        }

        if (paths.isNotEmpty()) {
            val relativePaths = paths.map { toRelative(it) }.toTypedArray()
            val addResult = cli.add(*relativePaths)
            if (!addResult.success) {
                errors.add(VcsException("Failed to stage files: ${addResult.stderr}"))
                return errors
            }
        }

        val result = cli.commit(commitMessage)
        if (!result.success) {
            errors.add(VcsException("Commit failed: ${result.stderr}"))
        }

        return errors
    }

    override fun getHelpId(): String? = null

    override fun isRefreshAfterCommitNeeded(): Boolean = true

    override fun scheduleMissingFileForDeletion(files: MutableList<out FilePath>): List<VcsException> {
        val errors = mutableListOf<VcsException>()
        for (file in files) {
            val result = cli.run("rm", toRelative(file.path))
            if (!result.success) {
                errors.add(VcsException("Failed to remove ${file.path}: ${result.stderr}"))
            }
        }
        return errors
    }

    override fun scheduleUnversionedFilesForAddition(files: MutableList<out VirtualFile>): List<VcsException> {
        val errors = mutableListOf<VcsException>()
        val paths = files.map { toRelative(it.path) }.toTypedArray()
        val result = cli.add(*paths)
        if (!result.success) {
            errors.add(VcsException("Failed to add files: ${result.stderr}"))
        }
        return errors
    }

    private fun toRelative(absolutePath: String): String {
        val basePath = project.basePath ?: return absolutePath
        return absolutePath.removePrefix(basePath).removePrefix("/").removePrefix("\\")
    }
}
