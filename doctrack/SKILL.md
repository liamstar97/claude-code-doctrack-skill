---
name: doctrack
description: >
  Maintains persistent codebase knowledge across sessions through structured feature and component
  documentation stored in an Obsidian vault via MCP tools. Use this skill whenever you have just
  made meaningful code changes (new features, modified components, refactoring, bug fixes) to update
  the project's documentation. Also use it when the user asks to document code, update docs, sync
  documentation, initialize documentation for an existing project, or when you want to understand
  the existing codebase structure at the start of a session. This skill should be used proactively
  after any significant code modification — don't wait for the user to ask. If you changed code,
  update the docs. Think of it as your long-term memory system: read before working, write after
  changing. Also use this when a user says "doctrack init", "initialize docs", "document this
  project", or wants to bootstrap documentation for a codebase that has no doctrack project note
  in Obsidian yet.
---

# Doctrack — Persistent Codebase Knowledge via Obsidian

You maintain project documentation in an **Obsidian vault** using the Obsidian MCP tools. The vault is the single source of truth — no documentation files are stored in the project repository (except `README.md` and `CLAUDE.md`).

Documentation is organized by **audience**:

- **`features/` and `components/`** — Your internal knowledge base. Dense, structured, optimized for you to quickly understand the codebase in future sessions. (Audience: Claude)
- **`guides/`** — Human-readable documentation. Clean, polished, organized for developers. (Audience: Human)
- **`specs/`** — Machine-readable specifications like OpenAPI. (Audience: Machine)

**This skill requires the Obsidian MCP server to be running.** All documentation operations use `mcp__obsidian__*` tools. If these tools are unavailable, inform the user that the Obsidian MCP connection is required.

## When to use this

**After making code changes**: Any time you add a feature, modify a component, refactor code, or fix a non-trivial bug, update the relevant documentation before finishing your response.

**At the start of a session**: Search Obsidian for the project's `_project.md` note to orient yourself. This is your memory from previous sessions — use it. If you find doctrack notes for this project, load the relevant ones before starting work.

**When the user asks**: If the user says anything about documenting, updating docs, syncing docs, or generating documentation.

**To initialize a project**: When the user says "doctrack init" or asks to document an existing project that has no doctrack project note in Obsidian yet. See the **Project Initialization** section below.

**Do NOT use this for**: Trivial formatting changes, comment-only edits, or when you're just reading/exploring code without modifying it.

## Vault structure

All paths below use `{prefix}` which resolves to:
- **Single-project vault**: empty (notes at vault root)
- **Shared vault**: `projects/{project-name}`

The vault layout is determined during `doctrack init` and stored in `_project.md` frontmatter.

### Single-project vault

```
vault-root/
├── _project.md                     # Project config — always read this first
├── features/
│   └── {feature-name}.md           # Feature overviews (audience: claude)
├── components/
│   └── {feature}/{component}.md    # Component details (audience: claude)
├── guides/
│   ├── architecture.md             # System architecture (audience: human)
│   ├── development.md              # Build/run/test (audience: human)
│   ├── api.md                      # API reference (audience: human)
│   └── {topic}.md                  # Topic guides (audience: human)
├── specs/
│   └── openapi.md                  # OpenAPI spec in code block (audience: machine)
├── references/
│   ├── imported/                   # Pre-existing docs imported during init
│   └── user/                       # User-provided reference materials
└── legacy/                         # Pre-existing docs preserved during init
```

### Shared (multi-project) vault

```
vault-root/
├── _doctrack.md                    # Global config — lists all tracked projects
└── projects/
    └── {project-name}/
        ├── _project.md
        ├── features/ components/ guides/ specs/ references/ legacy/
        └── (same structure as single-project)
```

### Monorepo (within a project)

```
{prefix}/
├── _project.md                     # Root config: package map, cross-package deps
├── packages/
│   └── {package-name}/
│       ├── _package.md             # Package-level config
│       ├── features/ components/ guides/ specs/
│       └── ...
├── guides/
│   └── architecture.md             # Root-level monorepo architecture
└── references/
```

### Reference notes (`{prefix}/references/`)

This directory holds supplementary documentation you should consult when working on the codebase:

- **`references/imported/`** — Existing project documentation that was present before doctrack was initialized (e.g., original README files, wiki exports, design docs, ADRs). During init, these are written to Obsidian for your internal use.

- **`references/user/`** — Documents the user has added for you to reference. These might be API docs for external services, design specs, style guides, compliance requirements, or anything else relevant to the project.

When starting a session, after reading `_project.md`, check for references: `mcp__obsidian__list_directory("{prefix}/references")`. These provide context not captured in feature/component docs — design rationale, external API contracts, business requirements, etc. Reference them in feature docs using wikilinks when relevant (e.g., "See [[references/user/stripe-api-guide|Stripe API Guide]] for webhook signature verification details").

## Tag taxonomy

Every doctrack-managed note gets **three required tags** applied via `mcp__obsidian__manage_tags`:

| Category | Tags | Purpose |
|----------|------|---------|
| **Type** | `doctrack/type/feature`, `doctrack/type/component`, `doctrack/type/guide`, `doctrack/type/reference`, `doctrack/type/spec`, `doctrack/type/index`, `doctrack/type/legacy` | What kind of doc |
| **Status** | `doctrack/status/active`, `doctrack/status/deprecated`, `doctrack/status/draft` | Current state |
| **Audience** | `doctrack/audience/claude`, `doctrack/audience/human`, `doctrack/audience/machine` | Who it's for |

Additional tags for multi-project and monorepo contexts:

| Category | Tags | When |
|----------|------|------|
| **Project** | `doctrack/project/{name}` | Shared vaults only |
| **Package** | `doctrack/package/{name}` | Monorepos only |

**Rule**: If you need to search/filter by it, make it a tag. If you need to read its value, put it in frontmatter. Tags and frontmatter complement each other.

## Step-by-step workflow

### 1. First session setup (first time only)

If the project has no doctrack notes in Obsidian yet and you're just making a single change (not initializing the whole project), create a minimal structure.

**Step 1: Determine vault layout**

Check if this is a shared or single-project vault:
1. Use `mcp__obsidian__list_directory("/")` to check for a `projects/` directory or `_doctrack.md` note.
2. If `_doctrack.md` exists or `projects/` exists → this is a shared vault. Set `{prefix}` to `projects/{project-name}`.
3. Otherwise → this is a single-project vault. Set `{prefix}` to empty (vault root).

**Step 2: Detect project name**

Extract the project name from `package.json` `name` field, the project directory name, or ask the user.

**Step 3: Create the project config note**

Use `mcp__obsidian__write_note` to create `{prefix}/_project.md` with this template:

```markdown
---
project: {project-name}
type: index
cwd: {filesystem-path-to-project}
vault_layout: single|shared
monorepo: false
initialized: YYYY-MM-DD
last_updated: YYYY-MM-DD
---

# {Project Name}

> Auto-maintained by doctrack. Last updated: YYYY-MM-DD

## Features

| Feature | Note | Description | Status | Last Updated |
|---------|------|-------------|--------|--------------|

## File Registry

| Source File | Feature | Component | Role |
|------------|---------|-----------|------|
```

Then tag it:
```
mcp__obsidian__manage_tags("{prefix}/_project.md", "add", ["doctrack/type/index", "doctrack/status/active", "doctrack/audience/claude"])
```

For shared vaults, also update or create `_doctrack.md` at vault root with the new project entry.

**Step 4: Write doctrack instructions to the project's `CLAUDE.md`**

Read the project's existing `CLAUDE.md` from the filesystem (if any) using the `Read` tool. If it already contains a `# Doctrack` section, replace that section in place using the `Edit` tool. If not, append the doctrack section to the end using the `Edit` tool (or create the file with `Write` if it doesn't exist).

Write this content (substituting the actual project name and vault prefix):

```markdown
# Doctrack

This project's documentation is maintained in an Obsidian vault using the doctrack skill.
Project: `{project-name}` | Vault path: `{prefix}/`

## Before starting work
- Search Obsidian for this project's docs: the project config note is at `{prefix}/_project.md`
- Read relevant feature and component notes to understand the area you'll be working in
- Check `{prefix}/references/` for any supplementary documentation
- Use the doctrack skill to load and review documentation before planning any implementation

## After making code changes
- Use the doctrack skill to update documentation for any code you modified
- Update the relevant feature/component notes in Obsidian
- Update the project config note (`{prefix}/_project.md`) if you added new features or files
- Update human-readable guides in `{prefix}/guides/` if the changes affect developer-facing documentation
- Always update `last_updated` frontmatter timestamps on modified notes
```

This ensures that future Claude Code sessions in this project will automatically discover and use doctrack documentation.

**Monorepo detection**: Check for monorepo indicators (`workspaces` in `package.json`, `pnpm-workspace.yaml`, `lerna.json`, `turbo.json`, `nx.json`). If detected, set `monorepo: true` in frontmatter and see the **Monorepo** sections below.

For full project initialization (documenting an entire existing codebase), see the **Project Initialization** section below instead.

### 2. Read existing documentation (session startup)

Use a search-first approach — don't rely on reading a single index file.

**Step 1: Find the project config**

Search for the project's config note:
```
mcp__obsidian__search_notes("{project-name}", searchFrontmatter=true, limit=5)
```
Or read it directly if you know the path:
```
mcp__obsidian__read_note("{prefix}/_project.md")
```

If the project config is not found, the project has not been initialized — prompt to run `doctrack init`.

**Step 2: Orient yourself**

```
mcp__obsidian__get_vault_stats(recentCount=10)
```
This shows recently modified notes, helping you identify what was worked on in previous sessions.

**Step 3: Load relevant context**

Based on what you're about to work on:
- If the user mentions specific files or features: `mcp__obsidian__search_notes("{feature-name}", searchFrontmatter=true)` to find relevant docs.
- For broader context: `mcp__obsidian__list_directory("{prefix}/features")` to see all features, then `mcp__obsidian__read_multiple_notes` on the relevant ones (up to 10 at a time).
- Check references: `mcp__obsidian__list_directory("{prefix}/references")` — scan for user-provided or imported docs relevant to your current task.

**In a monorepo**: Read the root `_project.md` for the package map. Determine which package you're working in from the current working directory. Read that package's `_package.md`, then load its relevant feature/component docs.

### 3. Update internal docs (features and components)

Be surgical — only update docs for code that actually changed. Don't rewrite everything.

#### Feature notes: `{prefix}/features/{feature-name}.md`

Create with `mcp__obsidian__write_note`, update with `mcp__obsidian__patch_note`. Use this template:

```markdown
---
feature: feature-name
type: feature
files:
  - src/path/to/file.ts
  - src/path/to/other.ts
last_updated: YYYY-MM-DD
status: active
---

# Feature Name

## Purpose
What this feature does and why it exists. Be specific — your future self needs to
understand this without reading the code.

## Architecture
How the feature is structured. Key design decisions and why they were made.
Data flow, state management approach, important patterns.

## Key Files
- `src/path/to/file.ts` — Main entry point, handles X
- `src/path/to/other.ts` — Utility functions for Y

## Dependencies
- **Internal**: [[features/auth|Authentication]], [[features/database|Database layer]]
- **External**: express, lodash

## API Surface
Key exports, endpoints, or interfaces that other parts of the codebase use.

## Notes
Anything important for future sessions: gotchas, technical debt, planned changes,
non-obvious behavior.
```

After creating a new feature note, always tag it:
```
mcp__obsidian__manage_tags("{prefix}/features/{feature-name}.md", "add", ["doctrack/type/feature", "doctrack/status/active", "doctrack/audience/claude"])
```

#### Component notes: `{prefix}/components/{feature}/{component}.md`

```markdown
---
feature: parent-feature-name
type: component
files:
  - src/path/to/component.ts
last_updated: YYYY-MM-DD
status: active
---

# Component Name

## Responsibility
Single-sentence description of what this component does.

## Key Files
- `src/path/to/component.ts:15-80` — Core logic
- `src/path/to/types.ts:5-20` — Type definitions

## Public API
```typescript
// Key exports, function signatures, or interface definitions
```

## Internal Logic
How it works internally. Important algorithms, state transitions, data transformations.
Be dense — this is for you, not humans.

## Relationships
- **Used by**: [[features/auth|Authentication]], [[components/auth/session-manager|Session Manager]]
- **Depends on**: [[features/database|Database layer]]

## Known Issues
- Any bugs, technical debt, or TODOs
```

After creating a new component note, tag it:
```
mcp__obsidian__manage_tags("{prefix}/components/{feature}/{component}.md", "add", ["doctrack/type/component", "doctrack/status/active", "doctrack/audience/claude"])
```

#### Update the project config

After creating or modifying any feature/component notes, update `{prefix}/_project.md`:
- Use `mcp__obsidian__patch_note` to add new rows to the Features table and File Registry
- Use `mcp__obsidian__update_frontmatter("{prefix}/_project.md", {last_updated: "YYYY-MM-DD"}, merge=true)` to update the timestamp

#### Metadata-only updates

When you only need to update timestamps or status without changing content:
```
mcp__obsidian__update_frontmatter(path, {last_updated: "YYYY-MM-DD"}, merge=true)
```

When checking if a doc might be stale without reading its full content:
```
mcp__obsidian__get_frontmatter(path)
```

### 4. Update human-readable docs (guides and README)

After updating internal docs, update the human-facing documentation. These are for developers — descriptive, visual, and immediately useful.

#### README files

**Every project and sub-project must have a `README.md` at its root in the project filesystem.** The README is the front page for humans and git hosting platforms (GitHub, GitLab). Use the standard `Write` tool (not Obsidian) to write this file. It should include:
- Project name and a concise description of what it does
- Tech stack and key dependencies
- Quick start: how to build, run, and test the project
- Note that detailed docs are maintained in the project's Obsidian vault
- For monorepos: the root README maps all sub-projects and links to each sub-project's README

#### Guide notes in Obsidian (`{prefix}/guides/`)

Create descriptive, feature-focused notes — not just a single architecture file. Each major feature or subsystem should get its own guide. These should be higher level than feature/component notes but still descriptive enough that a developer can understand how features work.

**What to include:**
- **Architecture overview** (`{prefix}/guides/architecture.md`): High-level system design, how features connect, data flow. Use ASCII art generously — flow diagrams, sequence diagrams, state machines, data flow charts. Avoid ASCII directory/file tree listings.
- **Feature guides** (`{prefix}/guides/{feature}.md`): One per major feature. Explain what it does, how it works, key mechanisms and patterns. Use diagrams and tables liberally. Include code examples only when they genuinely clarify something prose cannot.
- **API docs** (`{prefix}/guides/api.md`): If there are APIs, document endpoints and usage.
- **Build, run, and test** (`{prefix}/guides/development.md`): How to set up the dev environment, build, run, and test. Include actual commands.
- **Glossary** (`{prefix}/guides/glossary.md`): If the project has domain-specific terminology.

Create each guide with `mcp__obsidian__write_note` and tag it:
```
mcp__obsidian__manage_tags(path, "add", ["doctrack/type/guide", "doctrack/status/active", "doctrack/audience/human"])
```

**Style guidelines:**
- Polished and well-written — humans will read these
- Use clear headings, tables, and ASCII art diagrams liberally
- Include code examples sparingly — only when they genuinely clarify something
- Do NOT include ASCII directory/file tree listings
- Maintain a consistent tone across all guides
- Use `[[wikilinks]]` to cross-reference other notes in the vault

Only create/update guides that are relevant to the changes made. Don't create empty placeholder notes.

#### Spec notes in Obsidian (`{prefix}/specs/`)

If the project exposes REST APIs, maintain an OpenAPI spec as an Obsidian note:

```markdown
---
type: spec
format: openapi
last_updated: YYYY-MM-DD
status: active
---

# OpenAPI Specification

```yaml
openapi: "3.0.0"
info:
  title: ...
...
```
```

Tag it: `doctrack/type/spec`, `doctrack/status/active`, `doctrack/audience/machine`.

If the OpenAPI spec needs to be consumed by tools (Swagger UI, code generators, CI validation), also write it to the project filesystem at `docs/openapi.yaml` using the standard `Write` tool — but only if the user requests this. By default, specs live in Obsidian only.

When updating API endpoints, always update both `{prefix}/guides/api.md` and `{prefix}/specs/openapi.md` together.

### 5. Deprecation handling

When code is removed or replaced, use Obsidian's metadata tools:

1. Update frontmatter:
   ```
   mcp__obsidian__update_frontmatter(path, {status: "deprecated", deprecated_date: "YYYY-MM-DD", replaced_by: "new-feature-name"}, merge=true)
   ```

2. Update tags:
   ```
   mcp__obsidian__manage_tags(path, "remove", ["doctrack/status/active"])
   mcp__obsidian__manage_tags(path, "add", ["doctrack/status/deprecated"])
   ```

3. Add a deprecation notice to the content:
   ```
   mcp__obsidian__patch_note(path, "# Feature Name", "# Feature Name\n\n> **DEPRECATED** (YYYY-MM-DD): Replaced by [[features/new-feature|New Feature]]. Kept for historical context.")
   ```

4. Update `_project.md` to reflect the deprecated status.

Do NOT delete notes — they serve as historical context. Use deprecation instead.

## Naming conventions

- **Feature notes**: lowercase, kebab-case (e.g., `user-authentication.md`, `payment-processing.md`)
- **Component notes**: lowercase, kebab-case matching the component name (e.g., `token-validator.md`, `payment-gateway.md`)
- **Guide notes**: lowercase, kebab-case, descriptive (e.g., `getting-started.md`, `api-reference.md`)
- **Vault paths**: always use forward slashes, relative to vault root

## Important principles

1. **Read before writing.** Always search for existing notes before creating new ones. Use `mcp__obsidian__search_notes` to check. Prevent duplicates.

2. **Source paths are relative.** File paths in frontmatter (the `files` field) are always relative to the project's filesystem root. Vault paths are relative to vault root.

3. **Dense internal docs.** Your `features/` and `components/` notes are for you — pack them with information. Include the "why" behind decisions, not just the "what."

4. **Clean human docs.** The `guides/` notes are for developers — keep them readable, well-organized, and free of internal implementation noise.

5. **Incremental updates.** Don't rewrite entire notes when only one section changed. Use `mcp__obsidian__patch_note` for surgical edits.

6. **Timestamp everything.** Always use `mcp__obsidian__update_frontmatter` to update `last_updated` when modifying a note. This helps you prioritize which docs might be stale.

7. **Feature boundaries matter.** Think carefully about what constitutes a "feature" vs a "component." A feature is a cohesive unit of functionality (e.g., "authentication", "search"). A component is a distinct part within a feature (e.g., "token-validator", "search-indexer").

8. **Tag consistently.** Every note gets type + status + audience tags. In shared vaults, also project tags. In monorepos, also package tags. Apply tags immediately after creating a note.

9. **Search first.** Use `mcp__obsidian__search_notes` before `mcp__obsidian__list_directory` to find docs. Search is faster and more flexible, especially for large vaults.

10. **Obsidian is the source of truth.** No documentation files in the project repo except `README.md` and `CLAUDE.md` (and optional machine-readable specs if the user opts in). Everything else lives in Obsidian.

11. **Use wikilinks for cross-references.** Link between notes using `[[path/to/note|Display Text]]` syntax. This enables Obsidian's graph view and backlinks features, making the documentation navigable and interconnected.

---

## Project Initialization

Use this when a codebase already has significant code but no doctrack project note in Obsidian. The user might say "doctrack init", "initialize docs", "document this project", or similar.

This is fundamentally different from the incremental workflow above — you're documenting an entire existing codebase rather than updating docs after a single change.

### Pre-init: Vault configuration

Before documenting anything, determine the vault layout.

1. **Ask the user**: "Is this Obsidian vault shared across multiple projects, or dedicated to this one project?"
   - If shared: set `{prefix}` to `projects/{project-name}`. Check if `_doctrack.md` exists at vault root; create it if not.
   - If single-project: set `{prefix}` to empty (vault root).

2. **Detect project name**: From `package.json` `name` field, the directory name, or ask the user.

3. **Check for existing doctrack data**: `mcp__obsidian__search_notes("doctrack/type/index", searchFrontmatter=true)` to see if this project was already initialized. If found, warn the user and ask whether to re-initialize or abort.

### Init workflow

#### Phase 1: Discover the project structure

This phase reads the **source code on the filesystem** using standard tools (Read, Glob, Grep) — not Obsidian.

1. **Read config files** to understand the tech stack:
   - `package.json`, `Cargo.toml`, `pyproject.toml`, `go.mod`, etc.
   - Framework config files (next.config.js, vite.config.ts, etc.)
   - Look at the dependency list — it reveals what the project does
   - **Check for monorepo indicators**: `workspaces` in `package.json`, `pnpm-workspace.yaml`, `lerna.json`, `turbo.json`, `nx.json`, or a `packages/`/`apps/`/`services/` directory with multiple sub-projects. If this is a monorepo, follow the **Init for monorepos** section below.

2. **Map the directory tree** — use glob patterns to understand the layout:
   - `src/**/*.{ts,js,py,go,rs}` (or whatever the language is)
   - Look for natural groupings: `src/features/`, `src/modules/`, `src/routes/`, `src/components/`, etc.
   - Note test directories, config directories, scripts

3. **Import existing documentation** — look for any pre-existing docs in the project:
   - README files (`README.md`, `packages/*/README.md`)
   - Doc directories (`docs/`, `documentation/`, `wiki/`)
   - Architecture Decision Records (`adr/`, `decisions/`)
   - Design docs, API specs, runbooks
   - **Write each found doc to Obsidian**: `mcp__obsidian__write_note("{prefix}/references/imported/{filename}.md", content, frontmatter={type: "reference", source: "imported", original_path: "..."})`. Tag them with `doctrack/type/reference`, `doctrack/status/active`, `doctrack/audience/claude`.
   - **Preserve pre-existing docs for humans**: Also write them to `{prefix}/legacy/` and tag with `doctrack/type/legacy`, `doctrack/status/active`, `doctrack/audience/human`.
   - These are your source material for understanding the project and should be referenced from feature docs using wikilinks where relevant.

4. **Identify feature boundaries** — this is the critical thinking step. Look for:
   - Directory-based groupings (e.g., `src/auth/`, `src/payments/`)
   - Route files that reveal API structure
   - Domain models that reveal business concepts
   - If the project is flat (no clear directories), group by domain concept based on file names and imports

#### Phase 2: Document features in parallel

For large projects, use subagents to document features in parallel. Each subagent should:

1. Receive: the feature name, the list of files belonging to that feature, the doctrack templates, and the Obsidian vault prefix
2. Read all source files for that feature (from the filesystem using Read)
3. Write the feature note AND component notes to Obsidian following the templates
4. Tag all created notes with the appropriate doctrack tags
5. Return: the vault paths of notes created and the file registry entries

**Component notes are not optional.** A feature with multiple source files should have component notes — one for each distinct logical unit (a service class, a middleware, a data model, a utility module, etc.). The feature note describes the big picture; component notes describe the internals. If a feature has only one source file, a component note is not needed. But features with 2+ files almost always warrant components.

Spawn one subagent per feature. Give each subagent this context:
```
You are documenting the "{feature-name}" feature for doctrack in Obsidian.

Vault prefix: {prefix}
Files to analyze:
- {list of source files}

Other features in this project (for cross-referencing via [[wikilinks]]):
- {list of other feature names and their file paths}

Write these notes to Obsidian using mcp__obsidian__write_note:
1. {prefix}/features/{feature-name}.md (the feature overview)
2. {prefix}/components/{feature-name}/{component}.md (one per logical component)

After writing each note, tag it with mcp__obsidian__manage_tags:
- Feature notes: ["doctrack/type/feature", "doctrack/status/active", "doctrack/audience/claude"]
- Component notes: ["doctrack/type/component", "doctrack/status/active", "doctrack/audience/claude"]

You MUST create component notes for features with multiple source files. Each distinct
class, service, middleware, model, or utility module should get its own component note.
The feature note covers the big picture; component notes cover the internals of each piece.

Use the standard frontmatter format. Be thorough — this is the initial documentation
that future sessions will rely on. Focus on architecture, key decisions, file roles,
dependencies, and anything non-obvious about how the code works.

IMPORTANT: Use [[wikilinks]] for all cross-references to other features and components.
Trace imports and function calls to identify cross-feature dependencies. In the Dependencies
section, use [[features/{other-feature}|Display Name]] links. In component Relationships
sections, populate "Used by" and "Depends on" with wikilinks. These cross-references are
critical — they help future sessions understand how changes to one feature ripple through
others.
```

#### Phase 3: Build the project config, README, and human docs

After all feature notes are written:

1. **Write `{prefix}/_project.md`** using `mcp__obsidian__write_note` — aggregate all features, components, and file mappings into the project config note using the template from the Bootstrap section. Tag it with `doctrack/type/index`, `doctrack/status/active`, `doctrack/audience/claude`.

2. **Write `README.md`** at the project root **on the filesystem** using the standard `Write` tool — the front page for humans and git hosting. Include project name, description, tech stack, how to build/run/test, and a note that detailed documentation is in the Obsidian vault. For monorepos, also write a README for each sub-project.

3. **Write `CLAUDE.md`** at the project root **on the filesystem** — this wires up future Claude Code sessions to use doctrack. Read the existing `CLAUDE.md` first (if any). If it already contains a `# Doctrack` section, update that section in place. If not, append the section to the end (or create the file if it doesn't exist). Use this template:

   ```markdown
   # Doctrack

   This project's documentation is maintained in an Obsidian vault using the doctrack skill.
   Project: `{project-name}` | Vault path: `{prefix}/`

   ## Before starting work
   - Search Obsidian for this project's docs: the project config note is at `{prefix}/_project.md`
   - Read relevant feature and component notes to understand the area you'll be working in
   - Check `{prefix}/references/` for any supplementary documentation
   - Use the doctrack skill to load and review documentation before planning any implementation

   ## After making code changes
   - Use the doctrack skill to update documentation for any code you modified
   - Update the relevant feature/component notes in Obsidian
   - Update the project config note (`{prefix}/_project.md`) if you added new features or files
   - Update human-readable guides in `{prefix}/guides/` if the changes affect developer-facing documentation
   - Always update `last_updated` frontmatter timestamps on modified notes
   ```

   Use the standard `Read` tool to check for an existing file, then `Edit` to update or `Write` to create.

4. **Write `{prefix}/guides/architecture.md`** to Obsidian — a high-level overview of how the system fits together, with visual diagrams of data flow and feature interactions. Avoid ASCII directory trees. Tag: `doctrack/type/guide`, `doctrack/audience/human`.

5. **Write `{prefix}/guides/{feature}.md`** for each major feature — descriptive guides for developers. Higher level than feature notes but substantive. Use diagrams, tables, wikilinks to feature notes. Tag: `doctrack/type/guide`, `doctrack/audience/human`.

6. **Write `{prefix}/specs/openapi.md`** to Obsidian — if the project has REST API endpoints, generate an OpenAPI 3.0+ spec wrapped in a YAML code block. Tag: `doctrack/type/spec`, `doctrack/audience/machine`.

7. **Write `{prefix}/guides/api.md`** — human-readable API reference if the project has APIs.

8. **Write `{prefix}/guides/development.md`** — build, run, and test instructions with actual commands (if applicable).

9. **Write `{prefix}/guides/glossary.md`** if the project has domain-specific terminology.

For shared vaults, update `_doctrack.md` at vault root with the new project entry.

#### Phase 3.5: Verify cross-references

After all feature notes exist, do a cross-reference pass:

1. **Batch-read feature notes**: Use `mcp__obsidian__read_multiple_notes` (up to 10 at a time) to load all feature docs.

2. **Check every feature's Dependencies section** — does it list all the other features it imports from? Read the source files' import statements to verify. Ensure wikilinks are correct.

3. **Check component Relationships sections** — are "Used by" and "Depends on" populated with wikilinks? A component that imports from another feature's files should list that dependency.

4. **Fill in gaps** — use `mcp__obsidian__patch_note` to add missing cross-references. This is the most common gap in init documentation and it matters because future sessions use these cross-references to understand change impact.

#### Phase 4: Verify completeness

Check that every source file in the project appears in the File Registry:

1. `mcp__obsidian__list_directory("{prefix}/features")` to get all feature notes.
2. `mcp__obsidian__read_multiple_notes` in batches to collect all `files` entries from frontmatter.
3. Compare against the source files discovered in Phase 1.
4. Files not belonging to any feature likely indicate:
   - A feature you missed (create notes for it)
   - Utility/shared code (document as a "shared" or "common" feature)
   - Dead code (note it in `_project.md`)

### Init for monorepos

For monorepos with multiple packages/apps, use **separate folders per package within Obsidian** with a lightweight root-level coordination note.

**Detecting monorepos:** Look for `workspaces` in `package.json`, `pnpm-workspace.yaml`, `lerna.json`, Turborepo config (`turbo.json`), Nx config (`nx.json`), or multiple `go.mod` files. Also check for a `packages/`, `apps/`, or `services/` directory containing multiple sub-projects with their own config files.

#### Monorepo vault structure

```
{prefix}/
├── _project.md                     # Root: package map, cross-package deps, shared conventions
├── packages/
│   ├── api/
│   │   ├── _package.md             # Package config: feature list, file registry
│   │   ├── features/
│   │   │   └── {feature-name}.md
│   │   ├── components/
│   │   │   └── {feature}/{component}.md
│   │   ├── guides/
│   │   └── specs/
│   ├── web/
│   │   ├── _package.md
│   │   └── ...
│   └── shared/
│       ├── _package.md
│       └── ...
├── guides/                          # Root-level guides
│   └── architecture.md
└── references/
```

#### Root `_project.md` for monorepos

The root config is a coordination note. Use `mcp__obsidian__write_note` with this template:

```markdown
---
project: {project-name}
type: index
cwd: {filesystem-path-to-project}
vault_layout: single|shared
monorepo: true
initialized: YYYY-MM-DD
last_updated: YYYY-MM-DD
---

# {Project Name} — Monorepo

> Auto-maintained by doctrack. Last updated: YYYY-MM-DD

## Packages

| Package | Path | Description | Tech Stack |
|---------|------|-------------|------------|
| @repo/api | packages/api | REST API server | Express, TypeScript |
| @repo/web | packages/web | Frontend app | React, TypeScript |
| @repo/shared | packages/shared | Shared types and utils | TypeScript |

## Cross-Package Dependencies

| Package | Depends On | Relationship |
|---------|-----------|--------------|
| @repo/api | @repo/shared | Types, validation schemas, constants |
| @repo/web | @repo/shared | Types, constants |
| @repo/web | @repo/api | HTTP API consumer |

## Shared Conventions
- [List monorepo-wide patterns, build conventions, shared tooling, etc.]
```

#### Per-package `_package.md`

Each package gets the full doctrack structure. Use `mcp__obsidian__write_note` to create `{prefix}/packages/{name}/_package.md` with the same template as `_project.md` but scoped to that package. Tag all notes within a package with `doctrack/package/{package-name}`.

In each package's feature notes, reference other packages using wikilinks:
```markdown
## Dependencies
- **Internal**: [[packages/api/features/auth|Auth middleware]], [[packages/api/features/database|Database layer]]
- **Cross-package**: [[packages/shared/_package|@repo/shared]] (types, validation schemas)
- **External**: express, cors
```

#### Monorepo init workflow

1. **Detect monorepo structure** — identify packages/apps and their boundaries
2. **Write root `{prefix}/_project.md`** — map all packages, their relationships, and shared conventions
3. **Init each package independently** — run the standard Phase 1-4 init within each package's folder (`{prefix}/packages/{name}/`)
4. **Write root `{prefix}/guides/architecture.md`** — high-level overview of the entire monorepo
5. **Write `README.md`** at the monorepo root and per-package on the filesystem
6. **Write `CLAUDE.md`** at the monorepo root on the filesystem using the same idempotent strategy as the standard init (read existing, update or append the `# Doctrack` section). For monorepos, the root `CLAUDE.md` should reference the root `_project.md` and list all packages. Optionally write per-package `CLAUDE.md` files scoped to that package's vault prefix (`{prefix}/packages/{name}/`).
7. **Cross-reference pass** — ensure each package's notes correctly reference their cross-package dependencies with wikilinks
8. **Tag everything** — add `doctrack/package/{name}` tags to all notes within each package

Use subagents to init multiple packages in parallel — each package is independent.

#### Finding docs in a monorepo

When starting work in a monorepo, determine your scope:

- **Working on a specific package**: Read that package's `_package.md` first via `mcp__obsidian__read_note("{prefix}/packages/{name}/_package.md")`, then the root `_project.md` for cross-package context.
- **Working across packages**: Read the root `_project.md` first, then the relevant package configs.
- **Searching by package**: Use `mcp__obsidian__search_notes` — notes are tagged with `doctrack/package/{name}` for easy filtering.

---

## Working with teams (multi-agent / concurrent access)

When other agents or team skills use doctrack, all agents share the same Obsidian vault. Unlike filesystem-based docs in git worktrees, there is **one copy of each note** — no merge conflicts, but concurrent write risk.

### For team orchestrators

If you're building a skill that spawns team members, include these instructions when assigning tasks:

```
Documentation requirements:
- Search Obsidian for existing docs before starting work: search_notes("{feature-name}")
- After completing code changes, update the relevant feature/component notes in Obsidian
- Only update docs for code YOU changed — don't touch docs for other features
- When adding entries to _project.md, use write_note with mode: "append" to avoid overwriting
- Set frontmatter editing_agent while editing a doc, clear it when done
- Use the doctrack templates and always tag notes after creating them
```

### How concurrent access works

Obsidian is a shared data store — all agents see the same notes in real-time:

1. **No merge conflicts** — there is only one copy of each note
2. **Real-time visibility** — when one agent updates a note, others see it immediately
3. **Concurrent write risk** — two agents could overwrite each other's changes to the same note

### Minimizing conflicts

**Scope-based partitioning**: Each agent should only update notes for the features/components it is actively modifying. This directly prevents write conflicts.

**Append-only for the project config**: When multiple agents work simultaneously, they should use `mcp__obsidian__write_note` with `mode: "append"` to add new rows to `_project.md`, rather than rewriting it.

**Advisory locking**: Before updating a feature note, check `mcp__obsidian__get_frontmatter` for an `editing_agent` field. If set and recent, skip that note and leave a note for the reconciliation step. When you start editing:
```
mcp__obsidian__update_frontmatter(path, {editing_agent: "{agent-id}", editing_since: "YYYY-MM-DDTHH:MM:SS"}, merge=true)
```
Clear it when done:
```
mcp__obsidian__update_frontmatter(path, {editing_agent: null, editing_since: null}, merge=true)
```

### What team members should do

When an agent starts work:

1. **Search for relevant docs** — `mcp__obsidian__search_notes` to understand what's documented
2. **Read relevant feature/component notes** — get up to speed on the area you'll work in
3. **Do your code work**
4. **Update notes for what you changed** — create or edit feature/component notes as needed
5. **Append new entries to `_project.md`** — don't rewrite it, use append mode

### Post-task reconciliation

After all agents complete their work, the orchestrator should:

1. **Read `_project.md`** — consolidate any append-mode additions, deduplicate table rows
2. **Check for stale notes** — if code was deleted, mark related notes as deprecated
3. **Update timestamps** — `mcp__obsidian__update_frontmatter` with current date
4. **Clear advisory locks** — remove any remaining `editing_agent` fields
5. **Verify the File Registry** — ensure all source files are still mapped correctly

---

## Obsidian tool reference

Quick reference for which MCP tool to use in each situation:

| Tool | When to Use |
|------|-------------|
| `mcp__obsidian__search_notes` | Finding docs by feature name, file path, or content. Use `searchFrontmatter=true` for metadata queries. |
| `mcp__obsidian__read_note` | Reading a single known note by path. |
| `mcp__obsidian__read_multiple_notes` | Batch loading at session start or during cross-reference passes. Up to 10 notes per call. |
| `mcp__obsidian__get_frontmatter` | Checking metadata (timestamps, status, files) without loading full content. |
| `mcp__obsidian__get_notes_info` | Checking existence and timestamps for multiple notes. |
| `mcp__obsidian__write_note` | Creating new notes. Use `mode: "append"` when adding to `_project.md` concurrently. |
| `mcp__obsidian__patch_note` | Surgical edits — replacing specific strings in existing notes. Fails on ambiguous matches unless `replaceAll=true`. |
| `mcp__obsidian__update_frontmatter` | Updating metadata only (timestamps, status, file lists). Use `merge=true` to preserve existing fields. |
| `mcp__obsidian__manage_tags` | Adding/removing tags. Use `operation: "add"` after creating notes, `"remove"`/`"add"` pair for status changes. |
| `mcp__obsidian__list_directory` | Browsing vault structure, listing features/components/guides. |
| `mcp__obsidian__get_vault_stats` | Orientation at session start — vault size and recently modified notes. |
| `mcp__obsidian__move_note` | Renaming or reorganizing notes (e.g., renaming a feature). |
| `mcp__obsidian__delete_note` | Only for truly erroneous notes created by mistake. Prefer deprecation over deletion. |

---

## Vault setup

To use doctrack, you need:

1. **An Obsidian vault** — can be an existing vault or a new one dedicated to project documentation.
2. **The Obsidian MCP plugin** — configured and running so that Claude Code can access the vault via `mcp__obsidian__*` tools.
3. **Run `doctrack init`** — this handles all folder creation within the vault. No manual setup required.

The vault can be:
- **Dedicated** to a single project (simpler, all notes at vault root)
- **Shared** across multiple projects (notes under `projects/{name}/`)

You'll be asked which layout to use during `doctrack init`. This choice is stored in `_project.md` frontmatter and persists across sessions.
