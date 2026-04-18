# Setup Guide

> **For AI assistants:** You are being directed to set up foray for yourself. You know which MCP client you are (Claude CLI, Claude Desktop, Cursor, VS Code / GitHub Copilot). Determine the OS from system context, or ask the user. Then follow **Steps 2–4** for your client and OS. Step 1 is a one-time user action — confirm with the user whether the binary is already installed before attempting it.

Step 1 is done once. Steps 2–4 must be repeated for each AI assistant/client you want to use foray with — you can direct each assistant to this guide for further support.

## 1. Install the Binary (one-time)

Check if foray is already installed:

```sh
foray --version
```

If not installed, install via cargo:

```sh
cargo install foray
```

Or download a pre-built binary from [GitHub Releases](https://github.com/cavokz/foray/releases/latest):

| Platform | Binary |
|----------|--------|
| macOS (universal) | `foray-macos-universal` |
| Linux x86_64 | `foray-linux-x86_64` |
| Linux arm64 | `foray-linux-aarch64` |
| Windows x86_64 | `foray-windows-x86_64.exe` |
| Windows arm64 | `foray-windows-aarch64.exe` |

Place the binary somewhere on your `PATH` (e.g. `/usr/local/bin`, `~/.local/bin`).

<details>
<summary>Alternative: build from source</summary>

```sh
git clone https://github.com/cavokz/foray.git
cd foray
cargo install --path .
```

</details>

---

## Per-Assistant Setup (Steps 2–4)

Complete steps 2–4 for each AI assistant/client you use. You can direct each assistant to this guide for further support.

## 2. Configure MCP Server

The MCP server exposes foray's journal tools to your AI client. It runs locally as a subprocess — no network ports, no authentication.

After adding or changing MCP server config, restart the application or start a new chat session for the tools to appear.

### Claude CLI

Run this command to register foray as a global MCP server:

```sh
claude mcp add --scope user foray foray serve
```

### Claude Desktop

> **Note for Claude Desktop assistants:** You cannot configure your own MCP server — it is loaded before the session starts. Guide the user to edit the config file manually, then ask them to restart Claude Desktop before proceeding.

Config file:
- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "foray": {
      "command": "foray",
      "args": ["serve"]
    }
  }
}
```

### Cursor

Config file:
- macOS / Linux: `~/.cursor/mcp.json`
- Windows: `%USERPROFILE%\.cursor\mcp.json`

```json
{
  "mcpServers": {
    "foray": {
      "type": "stdio",
      "command": "foray",
      "args": ["serve"]
    }
  }
}
```

### VS Code / GitHub Copilot

Config file:
- macOS: `~/Library/Application Support/Code/User/mcp.json`
- Linux: `~/.config/Code/User/mcp.json`
- Windows: `%APPDATA%\Code\User\mcp.json`

```json
{
  "servers": {
    "foray": {
      "type": "stdio",
      "command": "foray",
      "args": ["serve"]
    }
  }
}
```

## 3. Install Companion Skill (Recommended)

The companion skill teaches your AI assistant when and how to use foray effectively.

For most clients, download `SKILL.md` from the [latest release](https://github.com/cavokz/foray/releases/latest/download/SKILL.md) and install it in the standard global path below. Some MCP clients may also surface built-in server instructions that mention a project-local skill location for certain clients; treat that as an optional project-scoped override, while the paths below are the recommended default install locations. For Claude Desktop, see the note after the table.

| Client | macOS | Linux | Windows |
|--------|-------|-------|---------|
| Claude CLI | `~/.claude/skills/foray/SKILL.md` | `~/.claude/skills/foray/SKILL.md` | `%USERPROFILE%\.claude\skills\foray\SKILL.md` |
| Claude Desktop | See note below | — | See note below |
| Cursor | `~/.cursor/skills/foray/SKILL.md` | `~/.cursor/skills/foray/SKILL.md` | `%USERPROFILE%\.cursor\skills\foray\SKILL.md` |
| VS Code / GitHub Copilot | `~/Library/Application Support/Code/User/prompts/foray.md` | `~/.config/Code/User/prompts/foray.md` | `%APPDATA%\Code\User\prompts\foray.md` |

> **Note for Claude Desktop assistants:** Claude Desktop has no file-based skill path. The practical alternative is project instructions: guide the user to open Claude Desktop, go to **Projects**, create or open the project they use for development work, click **"Set project instructions"** on the right panel, paste the full contents of `SKILL.md`, and click **"Save instructions"**. The skill will then be active for all conversations in that project. Projects and project instructions are available to all users including free accounts.

## 4. Verify

If you changed the MCP config in Step 2, make sure the application has been restarted or a new chat session has been started before continuing.

Invoke the `list_journals` MCP tool now. If the server is running correctly, it will respond (the list may be empty — that is fine).
