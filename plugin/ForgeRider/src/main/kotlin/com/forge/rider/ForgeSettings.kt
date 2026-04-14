package com.forge.rider

import com.intellij.openapi.components.*
import com.intellij.openapi.project.Project

@Service(Service.Level.PROJECT)
@State(
    name = "ForgeSettings",
    storages = [Storage("forge.xml")],
)
class ForgeSettings : PersistentStateComponent<ForgeSettings.State> {

    data class State(
        var executablePath: String = "forge",
    )

    private var state = State()

    var executablePath: String
        get() = state.executablePath
        set(value) { state.executablePath = value }

    override fun getState(): State = state

    override fun loadState(state: State) {
        this.state = state
    }

    companion object {
        fun getInstance(project: Project): ForgeSettings {
            return project.getService(ForgeSettings::class.java)
        }
    }
}
