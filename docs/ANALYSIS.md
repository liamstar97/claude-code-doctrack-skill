# Doctrack: repository analysis

*Written 2026-07-26 against `62df95b`. Findings marked **[fixed]** were resolved
in the commit that added this document; the rest are an open backlog.*

Standalone doctrack is frozen and superseded by Switchyard, so this analysis is
written with a specific bias: **the parts worth investing in are the parts that
migrate.** `dt-index` becomes the engine of the bundled doctrack plugin. Its bugs
travel with it; its tests protect it on arrival. Skill-authoring workflow and
distribution mechanics do not migrate and get proportionally less attention here.

---

## 1. What this repository is

Two loosely-coupled products under one roof.

**A Claude Code skill** (`skills/doctrack/SKILL.md`, ~1,000 lines of prose) that
tells Claude how to build and maintain a knowledge graph in an Obsidian vault at
`.doctrack/`. It defines eight node types, a controlled tag taxonomy, note
templates, a resumable four-phase init workflow, and migration paths between
schema versions. It performs no I/O itself — vault reads and writes are delegated
to the external obsidian skill (mcpvault).

**A Rust workspace** (~2,200 lines across four crates) that indexes the
relationship between source symbols and vault notes, and serves that index over
MCP and LSP.

```
dt-index ──┬── dt-watch ──┬── dt-mcp   (binary: doctrack-mcp)
           │              └── dt-lsp   (binary: doctrack-lsp)
           └──────────────────┘
```

| Crate | Lines | Role |
|-------|------:|------|
| `dt-index` | ~1,160 | Vault parsing, tree-sitter symbol extraction, matching, the index itself |
| `dt-mcp` | ~1,320 | 8 MCP tools, 6 one-shot CLI commands, Claude Code hook installation |
| `dt-lsp` | ~350 | Hover, go-to-definition, diagnostics |
| `dt-watch` | ~130 | Debounced file watching feeding incremental updates |

Plus `scripts/evaluate_vault.py` (815 lines) which scores a finished vault on
coverage, graph density, and content quality, and two committed benchmark runs
against an 18-module, 753-file Java monorepo.

### The central abstraction

`Index` holds five concurrent maps and answers two questions: *which notes
document this symbol* (`sym_to_docs`) and *which symbols does this note describe*
(`doc_to_syms`). `Index::build` populates them in four phases — walk the project
for a filename lookup, parse every vault note, extract symbols from every
parseable source file, then link.

Linking runs three tiers, weakest last:

| Tier | Confidence | Evidence |
|------|-----------|----------|
| 1 | `Exact` | A file reference that resolves to a real file |
| 2 | `Strong` | A backtick identifier matching a parsed symbol name |
| 3 | `Fuzzy` | Note title resembles a filename |

This tiering is the single best idea in the codebase, and §2 is largely the story
of it not being honoured.

---

## 2. Findings

Severity reflects how wrong the answer a user receives is, not how hard the fix
was.

### Critical

**C1. TypeScript was silently unsupported. [fixed]**
`ts_queries()` matched `class_declaration name: (identifier)`. In the TypeScript
grammar a class name is a `type_identifier`, so that pattern is not merely
unmatched — it is *structurally invalid*, and `tree_sitter::Query::new` rejected
the entire query. `extract_symbols` therefore returned `Err` for every `.ts` and
`.tsx` file, and `Index::build` logged the failure at `debug!` and moved on. A
TypeScript project produced an index of zero symbols with no visible error at any
log level a user would see. This shipped in v3.0.0 and was never noticed because
nothing in the repository ever asserted that a query compiles.

*Fixed:* corrected the query, added `every_language_query_compiles` plus
per-language extraction tests, and made a query-compilation failure carry an
explicit "this is a bug in doctrack" message rather than reading as a per-file
parse problem.

**C2. Incremental updates never rebuilt links. [fixed]**
`reindex_note` and `reindex_code_file` each carried a `// TODO: rebuild links`.
They replaced entries in `vault_notes`/`code_symbols` and left `sym_to_docs` and
`doc_to_syms` untouched. Consequences compounded:

- Every MCP tool calls `reindex_*` before answering, so *every* tool answered
  from links computed at process start.
- The file watcher called them on every save, so a long-lived MCP or LSP session
  drifted further from truth the longer it ran — the opposite of what a watcher
  is for.
- Renaming a symbol left its old `SymbolId` in `sym_to_docs` forever: the old
  name stayed "documented", the new one stayed invisible.
- Deletions removed the note from `vault_notes` but left its `DocLink`s behind,
  so `sym_to_docs` grew monotonically and reported notes that no longer existed.

*Fixed:* added `unlink_note`, `set_symbols`, `remove_note`, `remove_code_file`,
and `relink_notes_for_file`; `reindex_*` now unlink and relink. Six integration
tests cover rename, add, delete, and note-edit paths.

**C3. Fuzzy guesses were presented as documentation. [fixed]**
`SymbolRef` carried a `MatchConfidence`; `DocLink` did not. Since every consumer
reads `DocLink`, the confidence was thrown away exactly where it mattered.
`coverage_report`, `check_impact`, `refresh_docs`, `docs_for_file`, and LSP hover
all treated a title-similarity guess identically to an explicit frontmatter
reference.

The fuzzy tier was also badly calibrated: threshold 50 on a nucleo score, applied
to *every* file clearing it, linking *every symbol* in each. One note could
manufacture hundreds of links. Because coverage was computed as
`doc_to_syms.len() / vault_notes.len()`, this inflated the headline number toward
100% — the metric got better as the noise got worse.

*Fixed:* `DocLink` carries confidence; `is_verified()` and
`verified_docs_for_symbol()` gate every consumer that reports fact; the fuzzy
tier now requires a higher score, runs only when a note has no `Exact` link, and
takes only the single best-scoring file. Coverage reports verified and fuzzy
counts separately.

### High

**H1. Path matching used string `ends_with`. [fixed]**
`ref_str.ends_with(&filename)` in `docs_for_file`, `check_impact`, and
`--check-impact` meant a note referencing `auth.rs` matched `oauth.rs`, and
`session.rs` matched `mysession.rs`. Replaced with `ref_matches_path`, which
compares path components and requires a component-boundary suffix.

**H2. Symbol drift detection used substring containment. [fixed]**
`refresh_docs` decided a symbol was documented via
`body_lower.contains(&sym.name.to_lowercase())`. A symbol named `new` was
"documented" by the word *renewal*; `Index` by *indexing*; `get` by almost
anything. This suppresses real drift — the exact failure the tool exists to
catch. Replaced with `mentions_identifier`, which requires identifier boundaries.

**H3. Only files already referenced by notes were indexed. [fixed]**
`collect_referenced_code_files` gathered code files by resolving vault
references, so the symbol table was a subset of what the docs already mentioned.
This made `coverage_report`'s "undocumented code files" section *tautologically
unable* to list a file no note mentions — the headline gap-detection feature
could not detect the most important gap. It also made tier-2 matching nearly
redundant and left `resolve_symbol` blind to undocumented code.

*Fixed:* the whole project is indexed, bounded by `DOCTRACK_MAX_INDEXED_FILES`
(default 20,000) with referenced files prioritized and any truncation logged.
Because indexing everything raises tier-2 fan-out, a backtick identifier with
more than 8 definitions is now treated as too ambiguous to link, and identifiers
shorter than 3 characters are ignored.

**H4. Fenced code blocks were mined for references. [fixed]**
`extract_wikilinks` correctly stripped code blocks; `find_backtick_paths` and the
backtick-identifier scan did not. Every symbol in every Rust snippet and every
Mermaid node label became a `Strong` link. Given that SKILL.md mandates Mermaid
diagrams in nearly every note template, this was a large, systematic noise
source. All backtick-derived signals now run over fenced-block-stripped prose,
and identifiers are extracted once at parse time.

**H5. LSP broke on any path containing a space. [fixed]**
`PathBuf::from(uri.path())` was used in `hover`, `definition`, `did_open`,
`did_save`, and workspace-root resolution. `Uri::path()` is percent-encoded, so a
project at `/home/me/My Project` yielded `/home/me/My%20Project` and every
lookup missed silently. Replaced with `to_file_path()`.

### Medium

**M1. Duplicate links from a single note. [fixed]** Tier 1 pushed a `DocLink` per
(file_ref, symbol) pair with no dedup — a file named in frontmatter *and*
mentioned in prose produced two identical links, double-counting in every report.
Tiers 2 and 3 deduped; tier 1 did not.

**M2. A malformed flag started a server that hung. [fixed]** `--check-impact`
with no argument, or any typo'd flag, fell through the arg match and started the
MCP server on stdio — waiting forever for a handshake that would never arrive.
Now exits 2 with usage.

**M3. `--setup-hooks` panicked on unexpected `settings.json`. [fixed]** Four
`unwrap()`s on user-controlled JSON shape, and a parse failure silently
*discarded* the existing file rather than refusing. Now validates shape and
errors out rather than overwriting.

**M4. `.obsidian/` and `.trash/` were indexed as notes. [fixed]** `parse_vault`
walked every `.md` under the vault, including plugin READMEs in Obsidian's config
directory and notes the user had deleted. `evaluate_vault.py` already excluded
these; the Rust indexer didn't.

**M5. Index build blocked the async runtime. [fixed]** `DoctrackMcp::new` called
`index.build()` synchronously inside `#[tokio::main]`, delaying the MCP handshake
by the full build time on startup. Moved to `spawn_blocking`.

**M6. `resolve_symbol` had a hardcoded `.java` fallback. [fixed]** A leftover from
benchmarking against the Java monorepo: it looked up `format!("{}.java", name)`
in the file table, so the "not yet symbol-indexed" hint worked for exactly one
language. Now matches file stems across all languages.

**M7. Quadratic linking. [fixed]** Tier 2 looped over every symbol in every
indexed file for every backtick identifier in every note —
O(notes × identifiers × symbols). On the benchmark project's scale that is
~10⁸ string comparisons per build. Tier-3 fuzzy re-scored every file per note.
Fixed with a `symbol_names` map and identifiers precomputed at parse time.
`resolve_file_ref`'s partial-path fallback also scanned the entire file table on
every call; it now starts from the filename lookup.

**M8. Watcher failures took down the host process. [fixed]** Three `expect()`s in
a `spawn_blocking` closure. A watch failure — inotify limits, a deleted directory
— should degrade the index, not kill the MCP server.

**M9. The `_project.md` File Registry was invisible to the indexer. [fixed]**
SKILL.md instructs agents to maintain a markdown table mapping every source file
to its feature and component, and describes it as authoritative. Nothing parsed
it: `Frontmatter` read `file-registry` and `files` as YAML lists only. The most
explicit, highest-trust mapping in the entire vault was unused. Now parsed as a
`RegistryTable` source.

### Low

**L1. LSP advertised incremental sync it didn't implement. [fixed]**
`TextDocumentSyncKind::INCREMENTAL` with no `did_change` handler — clients
streamed per-keystroke diffs into the void. Set to `NONE`.

**L2. Go-to-definition ignored reference resolution. [fixed]** It joined
`file_ref.path` onto the project root directly instead of calling
`resolve_file_ref`, so bare filenames and partial paths — explicitly supported
everywhere else — never navigated.

**L3. `.gitignore` hid tracked files. [fixed]** `benchmarks/` was ignored while
two benchmark JSONs were committed, so `git status` would never surface a new
run. Narrowed to `benchmarks/local/`.

**L4. `CONTRIBUTING.md` described a repository that no longer exists. [fixed]**
It documented `doctrack/SKILL.md`, a `doctrack.skill` zip, and a
`doctrack-workspace/` directory — none of which are present — and made no mention
of the Rust workspace, which is two thirds of the codebase.

**L5. `README.md` omitted the entire v3 layer. [fixed]** No mention of
`doctrack-mcp`, `doctrack-lsp`, the MCP tools, or supported languages, and the
install command pointed at release artifacts of a differently-named repository.

**L6. No CI, and 15 clippy warnings. [fixed]** Nothing ran `cargo test`,
`clippy`, or `fmt`. Added a workflow covering all three plus a release build and
a smoke test of `evaluate_vault.py`.

**L7. Test coverage was 12 unit tests over pure helper functions. [fixed]**
Zero tests touched `Index::build`, `resolve_file_ref`, `link_all`,
`extract_symbols`, or any incremental path — which is precisely where C1, C2, C3,
H1, and H3 lived. Now 45 tests, including 17 fixture-based integration tests.

---

## 3. Open backlog

Not addressed, roughly in value order.

**B1. The project walk ignores `.gitignore`.** `build_file_lookup` uses a
hardcoded skip list (`target`, `node_modules`, `dist`, …) plus "any dotted
directory". Generated code, vendored trees, and build output outside those names
are indexed as first-class source. Switching to the `ignore` crate would fix this
and make the walk parallel for free. This is the highest-value remaining item.

**B2. `SymbolId` is `(file, name)`, so overloads collide.** Two Java methods
named `handle` in one class are one index entry. Adding a discriminator (start
line, or an occurrence counter) would separate them; every consumer that formats
a `SymbolId` would need updating.

**B3. MCP tools return prose strings.** Every tool returns pre-formatted
markdown, so the model re-parses English to act on it, and the shape is untested.
Returning structured content alongside the text would make the tools composable
and assertable.

**B4. Nothing tests `dt-mcp` or `dt-lsp`.** The index is now well covered, but
tool output formatting, hook installation, and LSP handlers have no tests. Hook
installation is the riskiest — it rewrites a user's `settings.json`.

**B5. The benchmark loop isn't reproducible.** Two runs are committed with
prose deltas, but reproducing one requires manually driving Claude through a full
init against a private 18-module repository. Whether these fixes improve real
vault quality is currently unmeasurable.

**B6. Known gaps from the last benchmark run** (`benchmarks/dojo-parent-run2.json`,
skill v2.0.0, still open): Maven multi-module and git-submodule monorepos are not
detected, which cascaded into module READMEs never being imported; 170 non-taxonomy
tags were created despite SKILL.md forbidding them; no OpenAPI specs were
generated for a REST-heavy project.

**B7. Version numbers disagree.** Crates are `0.1.0`, the skill declares `3.0.0`,
the README describes v3, and SKILL.md tells agents to compare `doctrack-mcp
--version` against `0.1.0`. It works, but nothing communicates compatibility.

**B8. `split_frontmatter` is fragile.** It finds the first `\n---` from byte 3,
so a `---` inside a YAML block scalar truncates the frontmatter, and a `----`
line matches the closing fence.

**B9. `evaluate_vault.py` is untested beyond a smoke test.** 815 lines of
scoring logic with no assertions on its output.

---

## 4. Notes for the Switchyard migration

Carried forward from this pass:

- **`dt-index` is the asset.** It is now the only part of the workspace with real
  test coverage, and it has no MCP or LSP knowledge. Port it whole.
- **Keep confidence tiering, and keep enforcing it.** The three-tier design was
  right; the failure was letting `Fuzzy` reach consumers unlabelled. Anything in
  the plugin that reports coverage or impact must go through
  `is_verified()`/`verified_docs_for_symbol()`.
- **Silent per-file error handling hid a total feature outage for a year.** C1
  survived because a compile-time-shaped failure was reported through a
  runtime-per-file channel at `debug!`. Distinguish "this file didn't parse" from
  "this language is broken", and make the latter loud.
- **Watch the incremental paths.** C2 is the kind of bug that only appears in a
  long-lived process, which is exactly what a plugin is. The relink tests should
  migrate alongside the code.
- **Coverage metrics need a denominator that isn't derived from the docs.** H3 is
  worth restating as a design principle: any "what's missing?" metric computed
  over a set defined by what's already documented will always report success.
