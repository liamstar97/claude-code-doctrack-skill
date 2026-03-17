# Doctrack

Persistent codebase knowledge for Claude. Doctrack maintains structured documentation that serves as Claude's long-term memory across sessions — read before working, write after changing.

## What it does

Doctrack creates and maintains two documentation trees:

- **`.claude_docs/`** — Claude's internal knowledge base. Dense, structured, optimized for quickly understanding the codebase in future sessions. Includes feature docs, component docs, a master index, and imported references.
- **`docs/`** — Human-readable documentation. Polished, visual, with ASCII art diagrams, feature guides, API references, and development instructions.

### Key capabilities

- **Proactive documentation** — Automatically updates docs after code changes without being asked
- **Project initialization** — Bootstrap full documentation for an existing codebase with `doctrack init`
- **Monorepo support** — Separate `.claude_docs/` per sub-project with a root coordination index for cross-package dependencies
- **Incremental updates** — Surgical doc updates scoped to what actually changed
- **Cross-references** — Tracks dependencies between features and components so you understand change impact
- **Pre-existing doc handling** — Imports existing docs into `references/imported/` and moves them to `docs/legacy/` to avoid duplication
- **Team/worktree support** — Merge-friendly structure for multi-agent workflows

## Installation

### Claude Code (CLI)

Install the skill globally so it's available in all projects:

```bash
claude install-skill /path/to/doctrack.skill
```

Or install from this repository directly:

```bash
claude install-skill https://github.com/liamstar97/claude-code-doctrack-skill/releases/latest/download/doctrack.skill
```

### Claude Desktop

1. Download `doctrack.skill` from the [latest release](https://github.com/liamstar97/claude-code-doctrack-skill/releases/latest)
2. Open Claude Desktop
3. Go to **Settings** > **Skills**
4. Click **Install Skill** and select the downloaded `.skill` file

## Usage

### Initialize a project

Say `doctrack init` to bootstrap documentation for an existing codebase. Doctrack will:

1. Analyze the project structure and tech stack
2. Identify features and their boundaries
3. Create `.claude_docs/` with feature docs, component docs, and a master index
4. Create `docs/` with architecture overview, feature guides, API reference, and development instructions
5. Create README files at every level
6. Import any pre-existing documentation

For monorepos, Doctrack detects workspace configurations and creates separate `.claude_docs/` per sub-project with a root coordination file mapping cross-package dependencies.

### After making code changes

Doctrack activates automatically after meaningful code changes. It will:

- Update the relevant feature and component docs in `.claude_docs/`
- Update affected human-readable docs in `docs/`
- Update the master index with new file mappings
- Keep cross-references current

### At the start of a session

When Claude starts working on a project with `.claude_docs/`, it reads the index first to orient itself — picking up context from previous sessions without re-reading the entire codebase.

## Documentation structure

### Standard projects

```
project/
├── .claude_docs/
│   ├── index.md                    # Master index
│   ├── references/
│   │   ├── imported/               # Pre-existing docs
│   │   └── user/                   # User-provided references
│   └── {feature}/
│       ├── feature.md              # Feature overview
│       └── components/
│           └── {component}.md      # Component detail
├── docs/
│   ├── legacy/                     # Pre-existing docs moved here
│   ├── architecture.md
│   ├── {feature}.md
│   ├── development.md
│   └── ...
└── README.md
```

### Monorepos

```
monorepo/
├── .claude_docs/
│   └── index.md                    # Root coordination file
├── packages/
│   ├── api/
│   │   ├── .claude_docs/           # Full doctrack structure
│   │   ├── docs/
│   │   └── README.md
│   ├── web/
│   │   ├── .claude_docs/
│   │   ├── docs/
│   │   └── README.md
│   └── shared/
│       ├── .claude_docs/
│       ├── docs/
│       └── README.md
├── docs/                           # Root-level docs
└── README.md
```

## How it works

Doctrack is a Claude Code skill — a markdown instruction file that guides Claude's behavior. When triggered, Claude follows the instructions in `SKILL.md` to read, create, or update documentation.

### Triggering

Doctrack triggers in three ways:

1. **Proactively** — After any meaningful code change, Claude updates the relevant docs
2. **On request** — When you say "doctrack init", "update docs", "document this", etc.
3. **On session start** — Claude reads `.claude_docs/index.md` to load context from previous sessions

### Internal docs (`.claude_docs/`)

These are Claude's notes — dense, structured, with YAML frontmatter for machine parsing. They track:

- **Features**: Cohesive units of functionality with purpose, architecture, dependencies, and API surface
- **Components**: Individual pieces within features — services, middleware, models, utilities
- **File registry**: Maps every source file to its feature and component
- **Cross-references**: Which features depend on which, so Claude understands change impact

### Human docs (`docs/`)

These are for developers — polished, visual, with:

- ASCII art diagrams (flow charts, sequence diagrams, state machines)
- Feature-specific guides explaining mechanisms and patterns
- API references and OpenAPI specs
- Build, run, and test instructions
- Glossary of domain-specific terms

## License

MIT
