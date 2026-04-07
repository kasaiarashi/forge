package com.forge.rider

import com.intellij.openapi.vcs.VcsKey
import com.intellij.openapi.vcs.VcsRootChecker

class ForgeRootChecker : VcsRootChecker() {

    override fun getSupportedVcs(): VcsKey {
        // VcsKey constructor is package-private in Kotlin, use Java interop
        val constructor = VcsKey::class.java.getDeclaredConstructor(String::class.java)
        constructor.isAccessible = true
        return constructor.newInstance("Forge")
    }

    override fun isVcsDir(dirName: String): Boolean {
        return dirName == ".forge"
    }
}
