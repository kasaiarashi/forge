package com.forge.rider

import com.intellij.openapi.options.Configurable
import com.intellij.openapi.progress.ProgressIndicator
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.Ref
import com.intellij.openapi.vcs.FilePath
import com.intellij.openapi.vcs.VcsException
import com.intellij.openapi.vcs.update.*

class ForgeUpdateEnvironment(
    private val project: Project,
    private val cli: ForgeCliRunner,
) : UpdateEnvironment {

    override fun fillGroups(updatedFiles: UpdatedFiles) {}

    override fun updateDirectories(
        contentRoots: Array<out FilePath>,
        updatedFiles: UpdatedFiles,
        indicator: ProgressIndicator,
        context: Ref<SequentialUpdatesContext>,
    ): UpdateSession {
        indicator.text = "Pulling from remote..."
        val result = cli.pull()

        return object : UpdateSession {
            override fun getExceptions(): List<VcsException> {
                return if (result.success) emptyList()
                else listOf(VcsException("Pull failed: ${result.stderr}"))
            }

            override fun onRefreshFilesCompleted() {}
            override fun isCanceled(): Boolean = false
        }
    }

    override fun createConfigurable(files: Collection<FilePath>): Configurable? = null

    override fun validateOptions(roots: Collection<FilePath>): Boolean = true
}
