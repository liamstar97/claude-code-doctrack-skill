> [!WARNING]
> **Standalone doctrack is frozen — EOL as of 2026-05-18.** It is superseded by
> **Switchyard**, where doctrack continues as a bundled first-party plugin.
> This repository remains available and installable but receives no new
> features. An existing `.doctrack/` vault migrates losslessly into
> `.switchyard/plugins/doctrack/`.

# Doctrack

A codebase knowledge graph for Claude. Doctrack builds and maintains a structured Obsidian vault (`.doctrack/`) that serves as persistent memory across sessions — read before working, write after changing.

## What it does

Doctrack creates a **knowledge graph** in a local Obsidian vault that travels with your code in git:

- **Features** — What the system does. High-level functional overviews.
- **Components** — How pieces work internally. Dense implementation details.
- **Concepts** — Cross-cutting patterns that span multiple features.
- **Decisions** — Why things are built this way, including rejected alternatives.
- **Interfaces** — Contracts and boundaries between features or packages.
- **Guides** — Procedural docs only: build, deploy, test, setup.

Notes are connected via `[[wikilinks]]` and visualized in Obsidian's graph view. Diagrams use Mermaid for token efficiency.

### Key capabilities

- **Proactive documentation** — Automatically updates docs after code changes
- **Knowledge graph** — Features, components, concepts, decisions, and interfaces form a navigable web of project knowledge
- **Local vault** — `.doctrack/` lives in your project directory and gets committed to git
- **Monorepo support** — Per-package documentation with cross-package concepts and interfaces
- **Decision tracking** — Records why decisions were made AND why alternatives were rejected
- **Incremental updates** — Surgical doc updates scoped to what actually changed
- **Team support** — Vault shared via git with advisory locking for concurrent access

## Installation

### Project-local (recommended)

Install for a single project. Shared with your team via git:

```bash
# From your project root
mkdir -p .claude/skills/doctrack
curl -L https://github.com/liamstar97/claude-code-doctrack-skill/releases/latest/download/doctrack.skill \
  -o /tmp/doctrack.skill && unzip -o /tmp/doctrack.skill -d .claude/skills/ && rm /tmp/doctrack.skill
```

Commit `.claude/skills/doctrack/` to your repo. Claude Code discovers project-local skills automatically.

### User-global

Install for all your projects:

```bash
mkdir -p ~/.claude/skills/doctrack
curl -L https://github.com/liamstar97/claude-code-doctrack-skill/releases/latest/download/doctrack.skill \
  -o /tmp/doctrack.skill && unzip -o /tmp/doctrack.skill -d ~/.claude/skills/ && rm /tmp/doctrack.skill
```

## Getting started

### 1. Initialize your project

```
> doctrack init
```

On first run, doctrack will:

1. **Install dependencies** — Installs the [obsidian skill](https://github.com/bitbonsai/mcpvault) (MCP server for vault operations) if not present
2. **Configure MCP** — Creates `.mcp.json` with the mcpvault server pointed at `.doctrack/`
3. **Create the vault** — Sets up `.doctrack/` with Obsidian config and `.gitignore`
4. **Analyze your codebase** — Reads config files, maps directory structure, identifies features
5. **Build the knowledge graph** — Creates feature, component, concept, decision, and interface notes
6. **Write project files** — `README.md`, `CLAUDE.md`, and procedural guides

> **Note**: After the first init, you may need to restart Claude Code for the MCP connection to activate. Run `doctrack init` again after restart to complete initialization.

### 2. Open in Obsidian (optional)

Open `.doctrack/` as a vault in Obsidian to browse the knowledge graph visually. The graph view shows how features, concepts, decisions, and interfaces connect.

### 3. Work normally

After initialization, doctrack activates automatically:

- **Session start** — Reads the vault to orient itself from previous sessions
- **After code changes** — Updates relevant features, components, and creates decision notes for non-trivial choices
- **Incremental** — Only touches docs for code that actually changed

## Vault structure

```text
project/
├── .doctrack/                      # Obsidian vault (committed to git)
│   ├── _project.md                 # Project config — read first
│   ├── features/                   # What the system does
│   ├── components/                 # How pieces work internally
│   ├── concepts/                   # Cross-cutting patterns
│   ├── decisions/                  # Why (and why not)
│   ├── interfaces/                 # Contracts between features
│   ├── guides/                     # Procedural docs (build, deploy, test)
│   ├── specs/                      # OpenAPI, schemas
│   └── references/                 # Imported pre-existing docs
├── .mcp.json                       # MCP server config (auto-generated)
├── README.md
└── CLAUDE.md                       # Wires up future sessions
```

### Monorepos

```text
.doctrack/
├── _project.md                     # Package map + cross-package deps
├── packages/
│   └── {name}/
│       ├── _package.md
│       ├── features/ components/ ...
│       └── ...
├── concepts/                       # Monorepo-wide patterns
├── decisions/                      # Monorepo-wide decisions
└── interfaces/                     # Cross-package contracts
```

## Dependencies

Doctrack depends on the **obsidian skill** ([bitbonsai/mcpvault](https://github.com/bitbonsai/mcpvault)) for vault operations. This is installed automatically during `doctrack init`. It provides:

- **MCP server** — Read/write/search/tag vault notes
- **Obsidian CLI** — Open vaults, trigger plugins, daily notes
- **Git sync** — Backup and sync vaults across devices

## How it works

Doctrack is two skills working together:

1. **Doctrack** (this skill) — Defines the knowledge graph schema: what notes to create, what frontmatter, what wikilinks, what tags. It's the brain that decides what to document.
2. **Obsidian skill** (mcpvault) — Handles the mechanics of reading and writing to the Obsidian vault via MCP tools. It's the hands that do the I/O.

When Claude starts a session, doctrack detects `.doctrack/`, reads the project config, and loads relevant context. When code changes, doctrack decides which notes to update and delegates the writes to the obsidian skill.

## For teams

The `.doctrack/` vault is committed to git, so the knowledge graph is shared with your team. When multiple agents or team members work concurrently:

- Each agent only updates notes for features it modifies
- Project config uses append-only mode to avoid conflicts
- Advisory locking via frontmatter prevents concurrent edits to the same note
- Post-task reconciliation consolidates changes

## License

MIT
