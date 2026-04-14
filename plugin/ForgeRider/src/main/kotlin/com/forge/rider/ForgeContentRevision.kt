package com.forge.rider

import com.intellij.openapi.vcs.FilePath
import com.intellij.openapi.vcs.changes.ContentRevision
import com.intellij.openapi.vcs.changes.CurrentContentRevision
import com.intellij.openapi.vcs.history.VcsRevisionNumber

/**
 * Represents a file at a specific Forge revision.
 * For now, content retrieval returns null (binary files are common in Forge).
 */
class ForgeContentRevision(
    private val filePath: FilePath,
    private val revision: String,
) : ContentRevision {

    override fun getContent(): String? {
        // Forge does not yet have a `forge show <revision>:<path>` command
        // that outputs file content. Return null for now.
        return null
    }

    override fun getFile(): FilePath = filePath

    override fun getRevisionNumber(): VcsRevisionNumber {
        return ForgeRevisionNumber(revision)
    }
}

class ForgeRevisionNumber(private val rev: String) : VcsRevisionNumber {

    override fun asString(): String = rev

    override fun compareTo(other: VcsRevisionNumber?): Int {
        return asString().compareTo(other?.asString() ?: "")
    }
}
