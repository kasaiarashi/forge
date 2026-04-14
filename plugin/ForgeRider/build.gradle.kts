plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm") version "1.9.25"
    id("org.jetbrains.intellij.platform") version "2.5.0"
}

group = "com.forge.rider"
version = "0.1.0"

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    intellijPlatform {
        intellijIdeaCommunity("2024.3")
    }
    implementation("com.google.code.gson:gson:2.11.0")
}

kotlin {
    jvmToolchain(21)
}

intellijPlatform {
    pluginConfiguration {
        id = "com.forge.rider"
        name = "Forge VCS"
        version = project.version.toString()
        description = """
            <p>Forge Version Control System integration for JetBrains IDEs.</p>
            <p>Binary-first VCS with file locking, designed for Unreal Engine game development.</p>
            <ul>
                <li>File status tracking (modified, added, deleted, untracked)</li>
                <li>Commit, push, and pull operations</li>
                <li>File locking (Perforce-style checkout/checkin)</li>
                <li>Branch management</li>
                <li>File history and diff</li>
                <li>Stash support</li>
            </ul>
        """.trimIndent()
        vendor {
            name = "Forge VCS"
        }
        ideaVersion {
            sinceBuild = "243"
        }
    }
}

tasks {
    buildSearchableOptions {
        enabled = false
    }
}
