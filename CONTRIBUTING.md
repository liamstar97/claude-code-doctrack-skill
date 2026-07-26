# Contributing to Doctrack

> [!NOTE]
> Standalone doctrack is frozen — see the warning in [README.md](README.md). It
> still accepts correctness fixes, tests, and documentation; the same code is
> carried forward into Switchyard's bundled doctrack plugin, so fixes here are
> not wasted.

## Repository structure

```
doctrack/
├── crates/
│   ├── dt-index/             # The code↔doc index. Everything else wraps this.
│   │   ├── src/vault.rs      #   Parse a note: frontmatter, refs, wikilinks, idents
│   │   ├── src/symbols.rs    #   Tree-sitter symbol extraction, per language
│   │   ├── src/matching.rs   #   Link notes to symbols across three confidence tiers
│   │   ├── src/index.rs      #   The maps, resolution, and incremental updates
│   │   └── tests/            #   Fixture-based end-to-end tests
│   ├── dt-mcp/               # MCP server + one-shot CLI (binary: doctrack-mcp)
│   ├── dt-lsp/               # LSP server for editors (binary: doctrack-lsp)
│   └── dt-watch/             # File watcher feeding incremental index updates
├── skills/doctrack/SKILL.md  # The skill — knowledge graph schema and workflows
├── scripts/evaluate_vault.py # Vault quality benchmarking
├── benchmarks/               # Recorded benchmark runs
├── docs/ANALYSIS.md          # Architecture and known-issue backlog
└── .github/workflows/ci.yml  # fmt, clippy, test
```

## Architecture

Doctrack has two halves that are worth keeping straight.

**The skill** (`skills/doctrack/SKILL.md`) defines the knowledge graph: what notes
to create, what frontmatter, what wikilinks, what tags. It never calls MCP tools
directly — it describes *what* to do and delegates the mechanics of reading and
writing notes to the obsidian skill ([bitbonsai/mcpvault](https://github.com/bitbonsai/mcpvault)).

**The Rust workspace** builds a bidirectional index between code symbols and
vault notes, and exposes it two ways: as MCP tools for Claude Code, and as an
LSP server for editors. The dependency order is
`dt-index ← dt-watch ← {dt-mcp, dt-lsp}`; `dt-index` has no knowledge of MCP or
LSP and should stay that way.

### How linking works

`Index::build` runs four phases: walk the project for a filename lookup table,
parse every vault note, extract symbols from every parseable source file, then
link notes to symbols. Links carry a `MatchConfidence`:

| Tier | Confidence | Source |
|------|-----------|--------|
| 1 | `Exact` | A file reference in frontmatter, a registry table, or a backtick path that resolves to a real file |
| 2 | `Strong` | A backtick identifier in prose matching a parsed symbol name |
| 3 | `Fuzzy` | Title-to-filename similarity, used only when the note has no `Exact` link |

**`Fuzzy` links are guesses.** Anything that reports documentation as fact —
coverage numbers, `check_impact`, "undocumented files" — must filter them out
with `MatchConfidence::is_verified()` or `Index::verified_docs_for_symbol`.

## Working on the Rust crates

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI runs all three plus a release build of both binaries. Keep it green.

### Adding a language

1. Add the `tree-sitter-*` grammar to `[workspace.dependencies]` and to
   `crates/dt-index/Cargo.toml`.
2. Add a `*_queries()` function and a `language_for()` arm in `symbols.rs`.
3. Add the extension to `SUPPORTED_EXTENSIONS`.
4. Add an `extracts_<language>_symbols` test.

Step 4 is not optional. A malformed tree-sitter query fails to compile at
runtime, which makes `extract_symbols` return `Err` for *every* file of that
language — and `Index::build` logs that at debug and carries on. TypeScript was
silently unsupported for exactly this reason until a test caught it. The
`every_language_query_compiles` test is the backstop.

### Testing against a real project

```bash
cargo build --release
export PATH="$PWD/target/release:$PATH"

cd /path/to/some/project
DOCTRACK_ROOT="$(pwd)" doctrack-mcp --coverage
DOCTRACK_ROOT="$(pwd)" doctrack-mcp --check-impact src/some/file.rs
DOCTRACK_ROOT="$(pwd)" doctrack-mcp --validate-note features/auth.md
doctrack-lsp --check "$(pwd)"          # full index dump, useful for debugging
```

Set `RUST_LOG=dt_index=debug` to see what the indexer skipped and why.

## Working on the skill

All skill logic lives in `skills/doctrack/SKILL.md`.

| Section | What it controls |
|---------|-----------------|
| Knowledge graph structure | Node types, wikilink patterns |
| Tag taxonomy | How notes are categorized |
| Session init | How Claude orients at session start |
| Note templates | Frontmatter and content structure per note type |
| Project initialization | The full init workflow |
| Version tracking | Migration paths between versions |

Tips:

- Explain "why", not just "what" — Claude follows instructions better with reasoning.
- Use templates and examples; they produce consistent output.
- Test on real codebases, not mock projects.

### Measuring skill changes

`scripts/evaluate_vault.py` scores a vault on coverage, graph density, and
content quality, and can diff against a previous run:

```bash
pip install pyyaml
python scripts/evaluate_vault.py /path/to/project -o benchmarks/myproject-run1.json
python scripts/evaluate_vault.py /path/to/project --compare benchmarks/myproject-run1.json
```

Committed runs live in `benchmarks/`. Record one before and after a substantive
skill change so the effect is visible rather than asserted.

## Areas for contribution

See [docs/ANALYSIS.md](docs/ANALYSIS.md) for the current backlog with severity
and rationale. Broad themes:

**Index quality** — confidence calibration, better ambiguity handling, respecting
`.gitignore` during the project walk.

**Knowledge graph** — new node types (risks, planned work), better concept
detection during init, extracting decisions from commit messages.

**Monorepo support** — Bazel and Pants detection, cross-package dependency
tracking. Note that Maven multi-module and git-submodule layouts were missed in
the last recorded benchmark run.

**Language coverage** — more grammars, framework-specific documentation patterns.

**Migration** — smoother v1/v2 → v3, and importing from JSDoc, Sphinx, and
similar.

## Submitting changes

1. Fork and branch.
2. Make the change. Add a test that fails without it.
3. `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
4. For skill changes, test against at least one real project and record a benchmark.
5. Open a PR describing what changed, why, and how you verified it.
