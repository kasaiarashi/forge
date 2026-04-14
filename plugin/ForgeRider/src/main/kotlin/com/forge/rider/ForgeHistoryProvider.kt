package com.forge.rider

import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.project.Project
import com.intellij.openapi.vcs.FilePath
import com.intellij.openapi.vcs.VcsException
import com.intellij.openapi.vcs.history.*
import java.util.Date
import javax.swing.JComponent

class ForgeHistoryProvider(
    private val project: Project,
    private val cli: ForgeCliRunner,
) : VcsHistoryProvider {

    override fun createSessionFor(filePath: FilePath): VcsHistorySession? {
        val relativePath = toRelative(filePath.path)
        val result = cli.log(file = relativePath)
        if (!result.success) return null

        val revisions = parseLog(result.stdout)
        return ForgeHistorySession(revisions)
    }

    override fun getAdditionalActions(refresher: Runnable): Array<AnAction>? = null

    override fun supportsHistoryForDirectories(): Boolean = false

    override fun isDateOmittable(): Boolean = false

    override fun getHelpId(): String? = null

    override fun getHistoryDiffHandler(): DiffFromHistoryHandler? = null

    override fun canShowHistoryFor(file: com.intellij.openapi.vfs.VirtualFile): Boolean = true

    override fun reportAppendableHistory(
        path: FilePath,
        partner: VcsAppendableHistorySessionPartner,
    ) {
        val relativePath = toRelative(path.path)
        val result = cli.log(file = relativePath)
        if (!result.success) {
            partner.reportException(VcsException("Failed to load history: ${result.stderr}"))
            return
        }
        val revisions = parseLog(result.stdout)
        for (rev in revisions) {
            partner.acceptRevision(rev)
        }
    }

    override fun getUICustomization(
        session: VcsHistorySession,
        component: JComponent,
    ): VcsDependentHistoryComponents {
        return VcsDependentHistoryComponents(emptyArray(), null, null)
    }

    private fun parseLog(output: String): List<VcsFileRevision> {
        if (output.isBlank()) return emptyList()

        val revisions = mutableListOf<VcsFileRevision>()
        var currentHash: String? = null
        var currentMessage: String? = null
        var currentAuthor: String? = null
        var currentDate: Date? = null

        for (line in output.lines()) {
            when {
                line.startsWith("commit ") -> {
                    if (currentHash != null) {
                        revisions.add(ForgeFileRevision(
                            ForgeRevisionNumber(currentHash),
                            currentMessage ?: "",
                            currentAuthor ?: "",
                            currentDate,
                        ))
                    }
                    currentHash = line.removePrefix("commit ").trim()
                    currentMessage = null
                    currentAuthor = null
                    currentDate = null
                }
                line.startsWith("Author: ") -> {
                    currentAuthor = line.removePrefix("Author: ").trim()
                }
                line.startsWith("Date: ") -> {
                    try {
                        val timestamp = line.removePrefix("Date: ").trim().toLong()
                        currentDate = Date(timestamp * 1000)
                    } catch (_: NumberFormatException) {}
                }
                line.startsWith("  ") && currentHash != null -> {
                    currentMessage = line.trim()
                }
            }
        }

        if (currentHash != null) {
            revisions.add(ForgeFileRevision(
                ForgeRevisionNumber(currentHash),
                currentMessage ?: "",
                currentAuthor ?: "",
                currentDate,
            ))
        }

        return revisions
    }

    private fun toRelative(absolutePath: String): String {
        val basePath = project.basePath ?: return absolutePath
        return absolutePath.removePrefix(basePath).removePrefix("/").removePrefix("\\")
    }
}

class ForgeFileRevision(
    private val revisionNumber: VcsRevisionNumber,
    private val message: String,
    private val author: String,
    private val date: Date?,
) : VcsFileRevision {

    override fun getRevisionNumber(): VcsRevisionNumber = revisionNumber
    override fun getRevisionDate(): Date? = date
    override fun getCommitMessage(): String = message
    override fun getAuthor(): String = author
    override fun getBranchName(): String? = null
    override fun getChangedRepositoryPath(): com.intellij.openapi.vcs.RepositoryLocation? = null
    override fun loadContent(): ByteArray? = null
    override fun getContent(): ByteArray? = null
}

class ForgeHistorySession(
    private val revisions: List<VcsFileRevision>,
) : VcsHistorySession {

    override fun getRevisionList(): MutableList<VcsFileRevision> = revisions.toMutableList()

    override fun getCurrentRevisionNumber(): VcsRevisionNumber? {
        return revisions.firstOrNull()?.revisionNumber
    }

    override fun shouldBeRefreshed(): Boolean = true

    override fun isContentAvailable(revision: VcsFileRevision): Boolean = false

    override fun hasLocalSource(): Boolean = true

    override fun isCurrentRevision(rev: VcsRevisionNumber?): Boolean {
        return rev?.asString() == currentRevisionNumber?.asString()
    }
}
