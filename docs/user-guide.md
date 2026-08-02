# IcePaw User Guide

Welcome to IcePaw -- your desktop AI assistant that puts privacy first. This guide walks you through everything from installation to power-user features.

---

## 1. Getting Started

### Installation

Download the latest installer from the [Releases page](https://github.com/your-org/ice-paw/releases):

| Platform | Installer |
|----------|-----------|
| Windows | `.msi` or `.exe` |
| macOS | `.dmg` |
| Linux | `.AppImage` or `.deb` |

Run the installer and launch IcePaw. The first launch creates a local SQLite database and an encrypted vault (called Stronghold) automatically -- no sign-up, no cloud account, no telemetry. Everything lives on your machine.

### Adding Your First API Key

IcePaw does not ship with built-in API keys. You bring your own. Before you can chat, you need to create an Agent and provide a key:

1. Click the **gear icon** in the top-right corner to open Settings.
2. Go to the **Agents** tab in the left sidebar.
3. Click the **"New Agent"** dashed card at the top.
4. Choose a **Provider** (OpenAI, Anthropic, DeepSeek, GLM, or MiniMax), enter your **API Key**, pick a **Model**, and give the agent an **ID** (e.g. `my-gpt`) and a **Name** (e.g. "My GPT-4o").
5. Click **Create**.

Your API key is encrypted with Stronghold and never stored as plaintext. It stays on your device and is only ever sent directly to the LLM provider you configured -- never to IcePaw or any third party.

---

## 2. Agents

An Agent is a configured AI persona -- think of it as a "profile" that bundles a provider, model, and behavior. You can create multiple agents and switch between them in the same window.

### Creating an Agent

Open **Settings > Agents** and click the dashed "New Agent" card. The form asks for:

- **Name** (display label, e.g. "Code Reviewer")
- **ID** (a unique slug, e.g. `code-reviewer` -- this cannot be changed after creation)
- **Provider** -- OpenAI, Anthropic, DeepSeek, GLM, or MiniMax (including their China-hosted endpoints)
- **Model** -- a dropdown of suggested models for your chosen provider, or type any model ID manually
- **API Key** -- your provider key (masked after entry)
- **API URL** (optional) -- leave blank to use the provider's default endpoint; fill in a custom base URL if you use a proxy or compatible third-party endpoint
- **Workspace Path** (optional) -- a local folder for the agent's configuration and knowledge base files

### Configuring Behavior via agent.yaml

Once an agent has a workspace path set, you can place an `agent.yaml` file in that folder. IcePaw auto-detects it and uses its contents for:

- `system_prompt` -- the system-level instruction that sets the agent's tone, role, and behavior
- `temperature` -- creativity control (0.0 = deterministic, 1.0 = creative)
- Any other provider-supported parameters passed through as `extra_params`

To change system prompt or temperature, simply edit the `agent.yaml` file in the agent's workspace folder. The settings card shows a green **agent.yaml** badge when the file is detected and active.

### Editing and Deleting Agents

Click any agent card to expand its inline editor. You can change the provider, model, API key, or base URL. To remove an agent, use the **More** menu (three dots) on the expanded card and confirm deletion. Existing conversations tied to that agent remain in your history.

---

## 3. Conversations

### Starting a Chat

1. In the left sidebar, select an **Agent** from the project space dropdown (or stay in "Scattered" for uncategorized chats).
2. Click the **+ New Conversation** button in the sidebar.
3. Type your message in the input box at the bottom and press **Enter** (or click Send).

The AI responds in a streaming fashion -- you see words appear as they are generated. If the agent invokes any tools (like reading a file or running a shell command), you will see each tool call and its result inline in the conversation.

### Managing Conversations

The sidebar lists your conversations, newest first. Each conversation shows its title and last-updated date. Right-click or use the actions next to each entry to:

- **Rename** -- click the title to edit it inline. Press Enter to confirm or Escape to cancel.
- **Pin** -- keep a conversation at the top of the list for quick access. Pinned conversations stay above unpinned ones.
- **Delete** -- remove a conversation permanently. This action cannot be undone.

### Searching Conversations

Use the search bar at the top of the sidebar to filter conversations by title. Type any part of the title to narrow the list in real time.

### Welcome State

If no conversation is currently selected (after deleting the last one, for instance), the main area shows a welcome screen. Pick any agent from the dropdown and start typing to begin a new chat. IcePaw also auto-restores the last active conversation when you launch the app.

---

## 4. Project Spaces

Project Spaces let you group related conversations and agents together. Think of them as workspaces for different topics or teams.

### Creating a Project

1. Click the project switcher at the top of the sidebar (the dropdown label, e.g. "Scattered Conversations").
2. Select **Manage Projects** to open the project list page.
3. Click **New Project** and fill in:
   - **Name** -- a display name like "Work" or "Side Project"
   - **Description** -- a short purpose statement
   - **Workspace Path** -- an optional folder for file tools to operate within
   - **Theme Color** -- an accent color for the project card (optional)
   - **Initial Agents** -- pick which agents belong to this project

### Using a Project

Once created, your project appears in the sidebar switcher dropdown. Select it to scope the conversation list to that project only. New conversations started while a project is active are automatically assigned to that project. Switching between projects keeps each one's conversation list independent.

### Adding Agents as Members

You can add agents to a project during creation or later. Agents assigned to a project appear in the project's settings. Each member agent can be used for conversations inside that project.

### Archiving and Restoring

Done with a project but not ready to delete it? Archive it.

- On the **Projects page**, click the archive action on a project card.
- The project moves to the **Archived** section at the bottom of the page. Its conversations remain intact and are hidden from the active conversation list.
- To restore, find the archived project and click **Restore**. Everything comes back exactly as it was.

Archiving is a soft delete -- your data stays safe. Only permanent deletion (with confirmation) actually removes the project and optionally its conversations.

---

## 5. MCP Tools

IcePaw supports the Model Context Protocol (MCP), which lets agents call external tools during a conversation. Tools are how an agent reads files, searches code, runs shell commands, and accesses the web.

### Built-in Tools

IcePaw ships with these tools, available to any agent:

| Tool | Description |
|------|-------------|
| `read_file` | Read a local file's contents |
| `list_directory` | List files and folders in a directory |
| `write_file` | Create or overwrite a file |
| `edit_file` | Perform exact string replacements in a file |
| `delete_file` | Delete a file or empty directory |
| `search_files` | Regex search across file contents (ripgrep) |
| `run_command` | Execute a shell command (requires per-use user approval) |
| `git` | Read-only git operations: status, diff, log, show |
| `web_fetch` | Fetch a URL's page content as markdown |
| `search_kb` | Search your knowledge base (see Section 6) |
| `read_kb_document` | Read a full knowledge base document |
| `save_to_kb` | Save content into your knowledge base |
| `read_agent_config` | Read the agent's own `agent.yaml` configuration |

### Permission Model

Tools use a three-tier authorization system:

1. **Always allowed** -- Safe read-only operations like git status and web fetch.
2. **Path-whitelisted** -- File operations restricted to the agent's or project's workspace folder.
3. **Require confirmation** -- Dangerous operations (like `run_command`) prompt you with a dialog showing the exact command before execution. You can approve or deny each one individually.

### Adding External MCP Servers

You can extend IcePaw with third-party MCP servers that provide additional tools. For example, you might add a server that gives agents access to a database, an issue tracker, or a custom API.

1. Open **Settings > Tools (MCP)**.
2. Click the dashed **New MCP Server** card.
3. Fill in:
   - **Name** -- a label like "PostgreSQL"
   - **Command** -- the executable to launch (e.g. `npx`, `python`, or a binary path)
   - **Arguments** -- CLI arguments (e.g. for an npx package: `-y @modelcontextprotocol/server-postgres`)
   - **Environment Variables** -- any env vars the server needs (API keys, connection strings)
   - **Trust Level** -- `trusted` for servers you control, `untrusted` to require per-tool confirmation
   - **Scope** -- `global` (all agents can use it) or `per_agent` (only agents that explicitly enable it)
4. Toggle the server **Enabled** and click **Save**.

External servers connect via stdio JSON-RPC. Their environment variables are filtered through a whitelist to prevent API key leakage. Active servers show a green "running" indicator; disabled or errored ones show "stopped" or "error".

---

## 6. Knowledge Base

The Knowledge Base (KB) feature lets you give agents awareness of your documents. Point IcePaw at a folder of markdown, text, or code files, and the agent can search and reference them during conversations.

### Setting Up a Knowledge Base

Knowledge bases are scoped to a level:

- **Global** -- available to all agents. Configured in **Settings > Knowledge Base**.
- **Agent-level** -- specific to one agent, located at `<agent_workspace>/kb/`.
- **Project-level** -- specific to a project, located at `<project_workspace>/kb/`.

The KB directory is derived automatically from the workspace path -- you don't configure it manually.

### How It Works

1. Place your documents (`.md`, `.txt`, `.json`, etc.) in the KB directory.
2. IcePaw watches the directory for changes and automatically re-indexes files.
3. When an agent needs information, it calls `search_kb` to find relevant documents by semantic similarity (using your agent's configured embedding model).
4. If a document matches, the agent can call `read_kb_document` to load its full content into context.
5. During a conversation, the agent can also call `save_to_kb` to persist important information for future reference.

### Reindexing

If you add, remove, or modify files in the KB directory, IcePaw detects the changes and updates the index automatically. You can also trigger a manual reindex from the KB settings page. The indexer parses file content, extracts metadata, computes content hashes for change detection, and stores summaries for fast retrieval.

---

## 7. Settings

Open Settings via the gear icon. The page has five tabs:

### General

- **Default Workspace Path** -- when a new agent is created without a workspace, its folder is auto-created here. Click the folder icon to browse or type a path manually.
- **Timezone** -- sets your local timezone. Click **Detect** to auto-detect from your system, or pick from the searchable IANA timezone list. Affects how message timestamps are displayed and is passed to the model as context.
- **Data Directory** -- read-only display of where your SQLite database, encrypted vault, and log files live. Click the folder icon to open it in your file manager.
- **Theme** -- toggle between Light and Dark mode. The toggle is located in the top bar for quick access; it animates with a smooth radial reveal transition.
- **Font Size** -- adjust the chat text size.
- **Language** -- set the UI display language.
- **Keyboard Shortcuts** -- record custom key combinations for common actions. Click a shortcut slot and press your desired keys to rebind.

### Agents

Create, edit, and delete agents. Each agent card shows its provider, model, workspace status, and whether it's reading from an `agent.yaml` file. See Section 2 for full details.

### Tools (MCP)

Manage built-in and external tool servers. The built-in tools section (collapsible) lists all thirteen system tools with descriptions. Below it, external MCP servers are shown as cards with status indicators. See Section 5 for full details.

### Knowledge Base

Manage the global knowledge base. Shows indexed documents, file paths, and allows reindexing. Agent-level and project-level KBs are managed from their respective agent or project settings. See Section 6 for full details.

### Logs

View application runtime logs with severity filtering. Logs are persisted to disk with daily rotation. Useful for troubleshooting connectivity issues, tool execution errors, or unexpected behavior.

---

## Quick Reference

| Task | Where |
|------|-------|
| Add an agent | Settings > Agents > New Agent |
| Start a new chat | Sidebar > + button |
| Pin a conversation | Right-click conversation > Pin |
| Rename a conversation | Click the title in the sidebar |
| Create a project | Sidebar dropdown > Manage Projects |
| Archive a project | Projects page > click archive |
| Add an MCP server | Settings > Tools > New MCP Server |
| Set up knowledge base | Put files in `<workspace>/kb/` |
| Toggle dark mode | Click the moon/sun icon in the top bar |
| Open data folder | Settings > General > Data Directory > folder icon |
| View logs | Settings > Logs |

---

## Data and Privacy

All your data stays on your machine:

| Item | Location |
|------|----------|
| Conversations & config | `ice-paw.db` (SQLite) |
| API keys | `stronghold.hold` (encrypted vault) |
| Application logs | Log files under the data directory |

To migrate to a new computer, copy the entire data directory. On Windows it is `%APPDATA%\com.icepaw.app\`; on macOS `~/Library/Application Support/com.icepaw.app/`; on Linux `~/.local/share/com.icepaw.app/`.

**API keys never leave your device except when making direct requests to the LLM provider you configured.** There is no telemetry, no analytics, and no cloud sync.

---

> Looking for developer documentation? See [Architecture](architecture.md) and [Backend API Reference](backend-api-reference.md).
