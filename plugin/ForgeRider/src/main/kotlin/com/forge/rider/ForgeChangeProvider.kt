package com.forge.rider

import com.intellij.openapi.progress.ProgressIndicator
import com.intellij.openapi.project.Project
import com.intellij.openapi.vcs.FilePath
import com.intellij.openapi.vcs.FileStatus
import com.intellij.openapi.vcs.changes.*
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.vcsUtil.VcsUtil
import java.io.File

class ForgeChangeProvider(
    private val project: Project,
    private val cli: ForgeCliRunner,
) : ChangeProvider {

    override fun getChanges(
        dirtyScope: VcsDirtyScope,
        builder: ChangelistBuilder,
        progress: ProgressIndicator,
        addGate: ChangeListManagerGate,
    ) {
        progress.text = "Scanning Forge workspace..."
        val status = cli.status()
        val root = project.basePath ?: return
        val vcsKey = ForgeVcs.getInstance(project).keyInstanceMethod

        // Staged new files
        for (path in status.stagedNew) {
            val filePath = toFilePath(root, path)
            val change = Change(null, CurrentContentRevision(filePath), FileStatus.ADDED)
            builder.processChange(change, vcsKey)
        }

        // Staged modified files
        for (path in status.stagedModified) {
            val filePath = toFilePath(root, path)
            val change = Change(
                ForgeContentRevision(filePath, "HEAD"),
                CurrentContentRevision(filePath),
                FileStatus.MODIFIED,
            )
            builder.processChange(change, vcsKey)
        }

        // Staged deleted files
        for (path in status.stagedDeleted) {
            val filePath = toFilePath(root, path)
            val change = Change(
                ForgeContentRevision(filePath, "HEAD"),
                null,
                FileStatus.DELETED,
            )
            builder.processChange(change, vcsKey)
        }

        // Unstaged modified files
        for (path in status.modified) {
            val vf = toVirtualFile(root, path) ?: continue
            builder.processModifiedWithoutCheckout(vf)
        }

        // Deleted files (unstaged)
        for (path in status.deleted) {
            val filePath = toFilePath(root, path)
            builder.processLocallyDeletedFile(filePath)
        }

        // Untracked files
        for (path in status.untracked) {
            val vf = toVirtualFile(root, path) ?: continue
            builder.processUnversionedFile(vf)
        }
    }

    override fun isModifiedDocumentTrackingRequired(): Boolean = false

    private fun toFilePath(root: String, relativePath: String): FilePath {
        val absPath = File(root, relativePath.replace('/', File.separatorChar))
        return VcsUtil.getFilePath(absPath)
    }

    private fun toVirtualFile(root: String, relativePath: String): VirtualFile? {
        val absPath = File(root, relativePath.replace('/', File.separatorChar))
        return LocalFileSystem.getInstance().findFileByIoFile(absPath)
    }
}
