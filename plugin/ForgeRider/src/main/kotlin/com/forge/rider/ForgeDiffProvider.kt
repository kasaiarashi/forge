package com.forge.rider

import com.intellij.openapi.project.Project
import com.intellij.openapi.vcs.FilePath
import com.intellij.openapi.vcs.changes.ContentRevision
import com.intellij.openapi.vcs.diff.DiffProvider
import com.intellij.openapi.vcs.diff.ItemLatestState
import com.intellij.openapi.vcs.history.VcsRevisionNumber
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.vcsUtil.VcsUtil

class ForgeDiffProvider(
    private val project: Project,
    private val cli: ForgeCliRunner,
) : DiffProvider {

    override fun getCurrentRevision(file: VirtualFile): VcsRevisionNumber {
        return ForgeRevisionNumber("working-copy")
    }

    override fun getLastRevision(file: VirtualFile): ItemLatestState? {
        return ItemLatestState(ForgeRevisionNumber("HEAD"), true, false)
    }

    override fun getLastRevision(filePath: FilePath): ItemLatestState? {
        return ItemLatestState(ForgeRevisionNumber("HEAD"), true, false)
    }

    override fun getLatestCommittedRevision(file: VirtualFile): VcsRevisionNumber? {
        return ForgeRevisionNumber("HEAD")
    }

    override fun createFileContent(revision: VcsRevisionNumber, file: VirtualFile): ContentRevision? {
        val filePath = VcsUtil.getFilePath(file)
        return ForgeContentRevision(filePath, revision.asString())
    }
}
