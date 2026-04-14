package com.forge.rider

import com.intellij.openapi.options.Configurable
import com.intellij.openapi.project.Project
import javax.swing.*

class ForgeConfigurable(private val project: Project) : Configurable {

    private var panel: JPanel? = null
    private var executableField: JTextField? = null

    override fun getDisplayName(): String = "Forge"

    override fun createComponent(): JComponent {
        val p = JPanel()
        p.layout = BoxLayout(p, BoxLayout.Y_AXIS)

        val row = JPanel()
        row.layout = BoxLayout(row, BoxLayout.X_AXIS)
        row.alignmentX = JPanel.LEFT_ALIGNMENT

        row.add(JLabel("Forge executable: "))
        val field = JTextField(getStoredPath(), 30)
        executableField = field
        row.add(field)

        p.add(row)

        val hint = JLabel("Path to the forge binary. Leave as \"forge\" to use PATH.")
        hint.alignmentX = JPanel.LEFT_ALIGNMENT
        hint.font = hint.font.deriveFont(hint.font.size2D - 1)
        p.add(Box.createVerticalStrut(4))
        p.add(hint)

        panel = p
        return p
    }

    override fun isModified(): Boolean {
        return executableField?.text != getStoredPath()
    }

    override fun apply() {
        val path = executableField?.text?.trim() ?: "forge"
        val vcs = ForgeVcs.getInstance(project)
        vcs.cli.forgeExecutable = path
        storeSettings(path)
    }

    override fun reset() {
        executableField?.text = getStoredPath()
    }

    private fun getStoredPath(): String {
        return project.getService(ForgeSettings::class.java)?.executablePath ?: "forge"
    }

    private fun storeSettings(path: String) {
        project.getService(ForgeSettings::class.java)?.executablePath = path
    }
}
