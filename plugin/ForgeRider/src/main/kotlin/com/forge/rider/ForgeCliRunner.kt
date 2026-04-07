package com.forge.rider

import com.google.gson.Gson
import com.google.gson.JsonObject
import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.execution.process.CapturingProcessHandler
import com.intellij.execution.process.ProcessOutput
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import java.io.File

data class CliResult(
    val exitCode: Int,
    val stdout: String,
    val stderr: String,
) {
    val success get() = exitCode == 0
}

data class ForgeStatus(
    val stagedNew: List<String> = emptyList(),
    val stagedModified: List<String> = emptyList(),
    val stagedDeleted: List<String> = emptyList(),
    val modified: List<String> = emptyList(),
    val deleted: List<String> = emptyList(),
    val untracked: List<String> = emptyList(),
    val locked: List<LockEntry> = emptyList(),
)

data class LockEntry(
    val path: String,
    val owner: String,
)

/**
 * Runs the `forge` CLI and parses output.
 * All VCS operations go through this runner.
 */
class ForgeCliRunner(private val project: Project) {

    private val log = Logger.getInstance(ForgeCliRunner::class.java)
    private val gson = Gson()

    /** Path to the forge executable. Defaults to "forge" (on PATH). */
    var forgeExecutable: String = "forge"

    private val workDir: File?
        get() = project.basePath?.let { File(it) }

    // ── Core execution ──

    fun run(vararg args: String): CliResult {
        return runCommand(false, *args)
    }

    fun runJson(vararg args: String): CliResult {
        return runCommand(true, *args)
    }

    private fun runCommand(json: Boolean, vararg args: String): CliResult {
        val cmd = GeneralCommandLine(forgeExecutable)
        if (json) cmd.addParameter("--json")
        args.forEach { cmd.addParameter(it) }
        cmd.workDirectory = workDir
        cmd.charset = Charsets.UTF_8

        return try {
            val handler = CapturingProcessHandler(cmd)
            val output: ProcessOutput = handler.runProcess(30_000)
            CliResult(
                exitCode = output.exitCode,
                stdout = output.stdout.trim(),
                stderr = output.stderr.trim(),
            )
        } catch (e: Exception) {
            log.warn("Failed to run forge: ${cmd.commandLineString}", e)
            CliResult(-1, "", e.message ?: "Unknown error")
        }
    }

    // ── Status ──

    fun status(): ForgeStatus {
        val result = runJson("status")
        if (!result.success || result.stdout.isBlank()) {
            return ForgeStatus()
        }
        return try {
            val obj = gson.fromJson(result.stdout, JsonObject::class.java)
            ForgeStatus(
                stagedNew = obj.getStringList("staged_new"),
                stagedModified = obj.getStringList("staged_modified"),
                stagedDeleted = obj.getStringList("staged_deleted"),
                modified = obj.getStringList("modified"),
                deleted = obj.getStringList("deleted"),
                untracked = obj.getStringList("untracked"),
                locked = obj.getAsJsonArray("locked")?.map {
                    val lock = it.asJsonObject
                    LockEntry(
                        path = lock.get("path")?.asString ?: "",
                        owner = lock.get("owner")?.asString ?: "",
                    )
                } ?: emptyList(),
            )
        } catch (e: Exception) {
            log.warn("Failed to parse forge status JSON", e)
            ForgeStatus()
        }
    }

    // ── Staging ──

    fun add(vararg paths: String): CliResult = run("add", *paths)
    fun unstage(vararg paths: String): CliResult = run("unstage", *paths)

    // ── Commit ──

    fun commit(message: String, all: Boolean = false): CliResult {
        val args = mutableListOf("snapshot", "-m", message)
        if (all) args.add("--all")
        return run(*args.toTypedArray())
    }

    // ── Push / Pull ──

    fun push(force: Boolean = false): CliResult {
        val args = mutableListOf("push")
        if (force) args.add("--force")
        return run(*args.toTypedArray())
    }

    fun pull(): CliResult = run("pull")

    // ── Locking ──

    fun lock(path: String, reason: String? = null): CliResult {
        val args = mutableListOf("lock", path)
        if (reason != null) {
            args.add("-r")
            args.add(reason)
        }
        return run(*args.toTypedArray())
    }

    fun unlock(path: String, force: Boolean = false): CliResult {
        val args = mutableListOf("unlock", path)
        if (force) args.add("--force")
        return run(*args.toTypedArray())
    }

    fun locks(): CliResult = run("locks")

    // ── Branching ──

    fun branch(): CliResult = run("branch")
    fun createBranch(name: String): CliResult = run("branch", name)
    fun deleteBranch(name: String): CliResult = run("branch", "-d", name)
    fun switch(name: String): CliResult = run("switch", name)

    // ── History ──

    fun log(count: Int = 50, file: String? = null): CliResult {
        val args = mutableListOf("log", "-n", count.toString())
        if (file != null) {
            args.add("--file")
            args.add(file)
        }
        return run(*args.toTypedArray())
    }

    // ── Diff ──

    fun diff(commitHash: String? = null): CliResult {
        val args = mutableListOf("diff")
        if (commitHash != null) {
            args.add("--commit")
            args.add(commitHash)
        }
        return run(*args.toTypedArray())
    }

    // ── Stash ──

    fun stash(message: String? = null): CliResult {
        val args = mutableListOf("stash")
        if (message != null) {
            args.add("-m")
            args.add(message)
        }
        return run(*args.toTypedArray())
    }

    fun stashPop(): CliResult = run("stash", "pop")
    fun stashList(): CliResult = run("stash", "list")

    // ── Restore / Revert ──

    fun restore(vararg paths: String): CliResult = run("restore", *paths)
    fun revert(commitHash: String): CliResult = run("revert", commitHash)

    // ── Workspace detection ──

    fun isForgeWorkspace(): Boolean {
        val dir = workDir ?: return false
        return File(dir, ".forge").isDirectory
    }

    // ── Helpers ──

    private fun JsonObject.getStringList(key: String): List<String> {
        return getAsJsonArray(key)?.map { it.asString } ?: emptyList()
    }
}
