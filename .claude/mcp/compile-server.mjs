#!/usr/bin/env node

/**
 * MCP Server: UE5 Compile Tool
 *
 * Exposes a `compile` tool to Claude Code that triggers the unified
 * compile.ps1 build script. Works with any UE5 project that has
 * compile.ps1 in its root.
 *
 * Protocol: MCP (Model Context Protocol) over stdio
 */

import { execSync } from "child_process";
import { createInterface } from "readline";
import { dirname, join } from "path";
import { fileURLToPath } from "url";

const PROJECT_ROOT = process.env.PROJECT_ROOT || process.cwd();

// JSON-RPC response helpers
function jsonrpcResponse(id, result) {
  return JSON.stringify({ jsonrpc: "2.0", id, result });
}

function jsonrpcError(id, code, message) {
  return JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } });
}

function handleRequest(request) {
  const { id, method, params } = request;

  switch (method) {
    case "initialize":
      return jsonrpcResponse(id, {
        protocolVersion: "2024-11-05",
        capabilities: { tools: {} },
        serverInfo: { name: "ue5-compile", version: "1.0.0" },
      });

    case "notifications/initialized":
      return null; // No response needed

    case "tools/list":
      return jsonrpcResponse(id, {
        tools: [
          {
            name: "compile",
            description:
              "Compile the Unreal Engine project. Auto-detects editor state: uses Live Coding if editor is open, falls back to UnrealBuildTool if closed. Returns compiler errors on failure.",
            inputSchema: {
              type: "object",
              properties: {},
              required: [],
            },
          },
        ],
      });

    case "tools/call": {
      const toolName = params?.name;
      if (toolName !== "compile") {
        return jsonrpcError(id, -32602, `Unknown tool: ${toolName}`);
      }

      try {
        const output = execSync(
          "powershell -ExecutionPolicy Bypass -File plugin/ForgeSourceControl/Scripts/compile.ps1",
          {
            cwd: PROJECT_ROOT,
            encoding: "utf-8",
            timeout: 600000, // 10 min
            stdio: ["pipe", "pipe", "pipe"],
          },
        );

        return jsonrpcResponse(id, {
          content: [{ type: "text", text: output.trim() }],
        });
      } catch (err) {
        // execSync throws on non-zero exit code
        const output = (err.stdout || "") + (err.stderr || "");
        return jsonrpcResponse(id, {
          content: [{ type: "text", text: output.trim() || "Build failed" }],
          isError: true,
        });
      }
    }

    case "ping":
      return jsonrpcResponse(id, {});

    default:
      // Ignore unknown methods (notifications etc)
      if (id !== undefined) {
        return jsonrpcError(id, -32601, `Method not found: ${method}`);
      }
      return null;
  }
}

// Stdio transport
const rl = createInterface({ input: process.stdin });

rl.on("line", (line) => {
  try {
    const request = JSON.parse(line);
    const response = handleRequest(request);
    if (response) {
      process.stdout.write(response + "\n");
    }
  } catch (e) {
    // Ignore parse errors
  }
});
