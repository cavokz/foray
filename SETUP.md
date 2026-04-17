# Setup Guide

This is a one-time setup guide. Read it and follow the steps to install foray and configure it for your MCP client.

## 1. Install the Binary

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

Verify installation:

```sh
foray --version
```

## 2. Configure MCP Server

foray uses **stdio** transport. The MCP client spawns `foray serve` as a child process and communicates over stdin/stdout. No network ports, no environment variables, no authentication.

| Setting | Value |
|---------|-------|
| Transport | `stdio` |
| Command | `foray` |
| Arguments | `["serve"]` |

If your client isn't listed below, use the table above to configure it.

### Claude Desktop (always global)

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

- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Linux**: `~/.config/Claude/claude_desktop_config.json`
- **Windows**: `%APPDATA%\Claude\claude_desktop_config.json`

### Cursor

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

- **Per-project**: `.cursor/mcp.json`
- **Global**: `~/.cursor/mcp.json`

> **Note**: After adding or changing MCP server config, start a new chat session for the tools to appear. Restarting the editor alone may not be enough.

### VS Code / GitHub Copilot

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

- **Per-project**: `.vscode/mcp.json`
- **Global**: VS Code Settings (JSON) → wrap the above in `"mcp": { ... }`

## 3. Install Companion Skill (Recommended)

The companion skill teaches your AI assistant when and how to use foray effectively.

Download `SKILL.md` from GitHub and save it to your project's skills directory:

- **VS Code**: `.github/copilot/skills/foray/SKILL.md`
- **Cursor**: `.cursor/skills/foray/SKILL.md`

Or save it globally if you want foray available in all projects.

## 4. Project Setup (manual step)

The CLI resolves journals via `.forayrc` files. To anchor a journal to a project, **`cd` to your project root** and run:

```sh
cd /path/to/your/project
foray open my-investigation --title "What I'm investigating"
```

This creates `.forayrc` in the current working directory — make sure you're in the right place. Optionally add `.forayrc` to `.gitignore`.

## 5. Verify

Call `list_journals` from your MCP client to confirm everything works. You should see the journal you just created.
