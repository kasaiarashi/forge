package com.forge.rider

import com.intellij.openapi.options.Configurable
import com.intellij.openapi.project.Project
import com.intellij.openapi.vcs.AbstractVcs
import com.intellij.openapi.vcs.VcsKey
import com.intellij.openapi.vcs.changes.ChangeProvider
import com.intellij.openapi.vcs.checkin.CheckinEnvironment
import com.intellij.openapi.vcs.diff.DiffProvider
import com.intellij.openapi.vcs.history.VcsHistoryProvider
import com.intellij.openapi.vcs.rollback.RollbackEnvironment
import com.intellij.openapi.vcs.update.UpdateEnvironment

class ForgeVcs(project: Project) : AbstractVcs(project, "Forge") {

    val cli = ForgeCliRunner(project)

    private val forgeChangeProvider = ForgeChangeProvider(project, cli)
    private val forgeCheckinEnvironment = ForgeCheckinEnvironment(project, cli)
    private val forgeUpdateEnvironment = ForgeUpdateEnvironment(project, cli)
    private val forgeRollbackEnvironment = ForgeRollbackEnvironment(project, cli)
    private val forgeDiffProvider = ForgeDiffProvider(project, cli)
    private val forgeHistoryProvider = ForgeHistoryProvider(project, cli)

    override fun getDisplayName(): String = "Forge"

    override fun getChangeProvider(): ChangeProvider = forgeChangeProvider

    override fun createCheckinEnvironment(): CheckinEnvironment = forgeCheckinEnvironment

    override fun createUpdateEnvironment(): UpdateEnvironment = forgeUpdateEnvironment

    override fun createRollbackEnvironment(): RollbackEnvironment = forgeRollbackEnvironment

    override fun getDiffProvider(): DiffProvider = forgeDiffProvider

    override fun getVcsHistoryProvider(): VcsHistoryProvider = forgeHistoryProvider

    override fun getConfigurable(): Configurable = ForgeConfigurable(myProject)

    companion object {
        fun getInstance(project: Project): ForgeVcs {
            val mgr = com.intellij.openapi.vcs.ProjectLevelVcsManager.getInstance(project)
            return mgr.findVcsByName("Forge") as ForgeVcs
        }

        fun getKey(project: Project): com.intellij.openapi.vcs.VcsKey {
            return getInstance(project).keyInstanceMethod
        }
    }
}
