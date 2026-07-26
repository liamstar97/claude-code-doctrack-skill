use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::info;

use dt_index::Index;

// --- Tool input schemas ---

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ValidateNoteInput {
    /// Path to the vault note to validate (relative to vault root, e.g. "Auth Flow.md")
    pub note: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DocsForFileInput {
    /// Path to a code file (relative to project root, e.g. "src/auth/session.rs")
    pub file: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ResolveSymbolInput {
    /// Symbol name to search for (e.g. "SessionManager", "authenticate")
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CheckImpactInput {
    /// Path to a code file that was modified (relative to project root)
    pub file: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchIndexInput {
    /// Search query — matches against note titles, symbol names, and file paths
    pub query: String,
}

// --- MCP Server ---

#[derive(Clone)]
pub struct DoctrackMcp {
    index: Arc<Index>,
    tool_router: ToolRouter<Self>,
}

impl DoctrackMcp {
    pub fn new(root: PathBuf, vault_root: PathBuf) -> Result<Self> {
        let index = Arc::new(Index::new(root.clone(), vault_root.clone()));

        if vault_root.exists() {
            // Building walks the project tree and parses every referenced source
            // file. On a large repo that is seconds of CPU, so keep it off the
            // async runtime — the MCP handshake must not wait on it.
            let build_index = index.clone();
            tokio::task::spawn_blocking(move || match build_index.build() {
                Ok(()) => info!(
                    "index ready: {} notes, {} code files, {} linked symbols",
                    build_index.vault_notes.len(),
                    build_index.code_symbols.len(),
                    build_index.sym_to_docs.len()
                ),
                Err(e) => info!("index build error: {e}"),
            });
        } else {
            info!(
                "no .doctrack/ vault found — tools will return empty results until vault is created"
            );
        }

        // Start file watcher in background
        let watcher_index = index.clone();
        let watcher_vault = vault_root.clone();
        let watcher_root = root.clone();
        tokio::spawn(async move {
            Self::run_watcher(watcher_index, watcher_vault, watcher_root).await;
        });

        Ok(Self {
            index,
            tool_router: Self::tool_router(),
        })
    }

    /// Run the file watcher, rebuilding index when vault appears or files change.
    async fn run_watcher(index: Arc<Index>, vault_root: PathBuf, project_root: PathBuf) {
        use dt_watch::FileWatcher;

        // If vault doesn't exist yet, poll until it appears
        if !vault_root.exists() {
            info!("waiting for .doctrack/ vault to be created...");
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if vault_root.exists() {
                    info!(".doctrack/ vault detected — building index");
                    if let Err(e) = index.build() {
                        info!("index build error on vault creation: {e}");
                    }
                    break;
                }
            }
        }

        let watcher = FileWatcher::new(index.clone(), vault_root, project_root);
        match watcher.start().await {
            Ok(mut rx) => {
                info!("file watcher started");
                while let Some(event) = rx.recv().await {
                    match &event {
                        dt_watch::WatchEvent::VaultNoteChanged(p) => {
                            info!("reindexed vault note: {}", p.display());
                        }
                        dt_watch::WatchEvent::CodeFileChanged(p) => {
                            info!("reindexed code file: {}", p.display());
                        }
                        dt_watch::WatchEvent::VaultNoteRemoved(p) => {
                            info!("removed vault note from index: {}", p.display());
                        }
                        dt_watch::WatchEvent::CodeFileRemoved(p) => {
                            info!("removed code file from index: {}", p.display());
                        }
                    }
                }
            }
            Err(e) => {
                info!("file watcher failed to start: {e}");
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DoctrackMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some("Doctrack MCP server. Provides bidirectional code↔documentation index tools for navigating and validating a .doctrack/ knowledge graph vault.".into());
        info
    }
}

#[tool_router(router = tool_router)]
impl DoctrackMcp {
    /// Validate a vault note for stale file references, broken wikilinks, and missing backlinks.
    #[tool(
        name = "validate_note",
        description = "Check a vault note for stale file references, broken wikilinks, and other issues. Use after writing or updating a note to ensure all references are valid."
    )]
    async fn validate_note(&self, Parameters(input): Parameters<ValidateNoteInput>) -> String {
        let note_path = self.index.vault_root.join(&input.note);

        // Re-index the note to get fresh data
        let _ = self.index.reindex_note(&note_path);

        let Some(note) = self.index.vault_notes.get(&note_path) else {
            return format!("Note not found: {}", input.note);
        };

        let mut issues = Vec::new();
        let mut resolved = 0;

        // Check file references
        for file_ref in &note.file_refs {
            let paths = self.index.resolve_file_ref(file_ref);
            if paths.is_empty() {
                issues.push(format!(
                    "STALE: `{}` — not found in project{}",
                    file_ref.path.display(),
                    if file_ref.is_bare_filename {
                        " (bare filename)"
                    } else {
                        ""
                    }
                ));
            } else if paths.len() > 1 {
                let locations: Vec<_> = paths
                    .iter()
                    .map(|p| {
                        p.strip_prefix(&self.index.root)
                            .unwrap_or(p)
                            .display()
                            .to_string()
                    })
                    .collect();
                issues.push(format!(
                    "AMBIGUOUS: `{}` resolves to {} files: {}",
                    file_ref.path.display(),
                    paths.len(),
                    locations.join(", ")
                ));
            } else {
                resolved += 1;
                // Check line validity
                if let Some(line) = file_ref.line
                    && let Ok(content) = std::fs::read_to_string(&paths[0])
                {
                    let line_count = content.lines().count() as u32;
                    if line > line_count {
                        issues.push(format!(
                            "STALE LINE: `{}:{}` — file only has {} lines",
                            file_ref.path.display(),
                            line,
                            line_count
                        ));
                    }
                }
            }
        }

        // Check wikilinks
        for link in &note.wikilinks {
            let linked_path = self.index.vault_root.join(format!("{link}.md"));
            if !linked_path.exists() {
                let found = self
                    .index
                    .vault_notes
                    .iter()
                    .any(|entry| entry.value().title.eq_ignore_ascii_case(link));
                if !found {
                    issues.push(format!(
                        "BROKEN WIKILINK: [[{link}]] — note not found in vault"
                    ));
                }
            }
        }

        if issues.is_empty() {
            format!(
                "✓ {} — all references valid ({} file refs resolved, {} wikilinks OK)",
                input.note,
                resolved,
                note.wikilinks.len()
            )
        } else {
            format!(
                "⚠ {} — {} issue(s) found:\n{}",
                input.note,
                issues.len(),
                issues
                    .iter()
                    .map(|i| format!("  - {i}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
    }

    /// Get all documentation notes that reference a given code file.
    #[tool(
        name = "docs_for_file",
        description = "Find all vault notes that document or reference a specific code file. Use when you need context about what a file does or want to check if documentation exists for it."
    )]
    async fn docs_for_file(&self, Parameters(input): Parameters<DocsForFileInput>) -> String {
        let abs_path = self.index.root.join(&input.file);

        // Try to resolve via file lookup if direct path doesn't exist
        let resolved_paths = if abs_path.exists() {
            vec![abs_path.clone()]
        } else {
            // Maybe it's a bare filename or partial path
            let filename = std::path::Path::new(&input.file)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Some(paths) = self.index.file_lookup.get(&filename) {
                paths.value().clone()
            } else {
                vec![]
            }
        };

        if resolved_paths.is_empty() {
            return format!("File `{}` not found in project", input.file);
        }

        let mut doc_links = Vec::new();
        let mut possible = Vec::new();
        let mut seen_titles = std::collections::HashSet::new();

        for path in &resolved_paths {
            // Ensure the file is indexed
            let _ = self.index.reindex_code_file(path);

            // Check symbol→doc links. Fuzzy title guesses are reported
            // separately so they can't be mistaken for real documentation.
            if let Some(symbols) = self.index.code_symbols.get(path) {
                for sym in symbols.value() {
                    for doc in self.index.docs_for_symbol(path, &sym.name) {
                        if !seen_titles.insert(doc.note_title.clone()) {
                            continue;
                        }
                        let line = format!(
                            "- **{}** [{}] documents `{}` ({})\n  {}",
                            doc.note_title, doc.note_type, sym.name, sym.kind, doc.context
                        );
                        if doc.confidence.is_verified() {
                            doc_links.push(line);
                        } else {
                            possible.push(line);
                        }
                    }
                }
            }

            // Also find notes that reference this file path directly
            for entry in self.index.vault_notes.iter() {
                let note = entry.value();
                for file_ref in &note.file_refs {
                    if dt_index::vault::ref_matches_path(&file_ref.path, path)
                        && seen_titles.insert(note.title.clone())
                    {
                        doc_links.push(format!(
                            "- **{}** [{}] references this file directly",
                            note.title, note.note_type
                        ));
                    }
                }
            }
        }

        let rel = resolved_paths
            .first()
            .and_then(|p| p.strip_prefix(&self.index.root).ok())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| input.file.clone());

        let mut out = if doc_links.is_empty() {
            format!("No documentation found for `{rel}`. Consider creating a vault note for it.")
        } else {
            format!("Documentation for `{}`:\n\n{}", rel, doc_links.join("\n"))
        };

        if !possible.is_empty() {
            out.push_str(&format!(
                "\n\nPossibly related (fuzzy title match — verify before relying on these):\n\n{}",
                possible.join("\n")
            ));
        }

        out
    }

    /// Find where a symbol (function, class, struct, etc.) is defined across the codebase.
    #[tool(
        name = "resolve_symbol",
        description = "Look up where a symbol is defined in the codebase and which vault notes reference it. Use when writing documentation to get accurate file paths and line numbers for a symbol."
    )]
    async fn resolve_symbol(&self, Parameters(input): Parameters<ResolveSymbolInput>) -> String {
        let mut results = Vec::new();

        for id in self.index.definitions_of(&input.name) {
            let Some(symbols) = self.index.code_symbols.get(&id.file) else {
                continue;
            };
            let rel_path = id
                .file
                .strip_prefix(&self.index.root)
                .unwrap_or(&id.file)
                .display();
            for sym in symbols.value().iter().filter(|s| s.name == input.name) {
                let docs = self.index.verified_docs_for_symbol(&id.file, &sym.name);
                let doc_info = if docs.is_empty() {
                    "no documentation".to_string()
                } else {
                    let titles: Vec<_> = docs.iter().map(|d| d.note_title.as_str()).collect();
                    format!("documented in: {}", titles.join(", "))
                };
                results.push(format!(
                    "- {} `{}` at `{}` (L{}-L{}) — {}",
                    sym.kind,
                    sym.name,
                    rel_path,
                    sym.start_line + 1,
                    sym.end_line + 1,
                    doc_info
                ));
            }
        }

        // A file named after the symbol is a strong hint even when we couldn't
        // parse symbols out of it (unsupported language, parse error).
        for entry in self.index.file_lookup.iter() {
            let stem = std::path::Path::new(entry.key())
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if stem != input.name {
                continue;
            }
            for path in entry.value() {
                if self.index.code_symbols.contains_key(path) {
                    continue;
                }
                let rel = path
                    .strip_prefix(&self.index.root)
                    .unwrap_or(path)
                    .display();
                results.push(format!("- file `{rel}` (not yet symbol-indexed)"));
            }
        }
        results.sort();

        if results.is_empty() {
            format!("Symbol `{}` not found in indexed code files", input.name)
        } else {
            format!("Found `{}`:\n\n{}", input.name, results.join("\n"))
        }
    }

    /// Check which vault notes are impacted by changes to a code file.
    #[tool(
        name = "check_impact",
        description = "After modifying a code file, check which vault notes reference it and may need updating. Use after renaming functions, moving files, or making significant code changes."
    )]
    async fn check_impact(&self, Parameters(input): Parameters<CheckImpactInput>) -> String {
        let abs_path = self.index.root.join(&input.file);

        // Re-index to pick up changes
        let _ = self.index.reindex_code_file(&abs_path);

        let Some(symbols) = self.index.code_symbols.get(&abs_path) else {
            return format!("No symbols found in `{}`", input.file);
        };

        let mut impacted = Vec::new();
        for sym in symbols.value() {
            for doc in self.index.verified_docs_for_symbol(&abs_path, &sym.name) {
                let line = format!(
                    "- **{}** [{}] references `{}`",
                    doc.note_title, doc.note_type, sym.name
                );
                if !impacted.contains(&line) {
                    impacted.push(line);
                }
            }
        }

        // Also check for notes that reference the file path directly
        for entry in self.index.vault_notes.iter() {
            let note = entry.value();
            for file_ref in &note.file_refs {
                if !dt_index::vault::ref_matches_path(&file_ref.path, &abs_path) {
                    continue;
                }
                let line = format!(
                    "- **{}** [{}] has file reference to `{}`",
                    note.title,
                    note.note_type,
                    file_ref.path.display()
                );
                if !impacted.contains(&line) {
                    impacted.push(line);
                }
            }
        }

        if impacted.is_empty() {
            format!(
                "No vault notes reference `{}` — no documentation impact.",
                input.file
            )
        } else {
            format!(
                "Changes to `{}` may affect {} vault note(s):\n\n{}",
                input.file,
                impacted.len(),
                impacted.join("\n")
            )
        }
    }

    /// Get a coverage report showing which code files lack documentation.
    #[tool(
        name = "coverage_report",
        description = "Get a summary of documentation coverage — which files are documented, which aren't, and overall vault health. Use to identify gaps in documentation."
    )]
    async fn coverage_report(&self) -> String {
        let total_notes = self.index.vault_notes.len();
        let total_code_files = self.index.code_symbols.len();
        // Only notes with at least one verified link count as linked — a fuzzy
        // title guess is not evidence that a note documents anything.
        let linked_notes = self
            .index
            .doc_to_syms
            .iter()
            .filter(|e| e.value().iter().any(|r| r.confidence.is_verified()))
            .count();
        let total_links: usize = self
            .index
            .sym_to_docs
            .iter()
            .map(|e| {
                e.value()
                    .iter()
                    .filter(|d| d.confidence.is_verified())
                    .count()
            })
            .sum();
        let fuzzy_links: usize = self
            .index
            .sym_to_docs
            .iter()
            .map(|e| {
                e.value()
                    .iter()
                    .filter(|d| !d.confidence.is_verified())
                    .count()
            })
            .sum();

        // Find undocumented code files
        let mut undocumented = Vec::new();
        for entry in self.index.code_symbols.iter() {
            let file = entry.key();
            let has_docs = entry.value().iter().any(|sym| {
                !self
                    .index
                    .verified_docs_for_symbol(file, &sym.name)
                    .is_empty()
            });
            if !has_docs {
                let rel = file.strip_prefix(&self.index.root).unwrap_or(file);
                undocumented.push(rel.display().to_string());
            }
        }
        undocumented.sort();

        // Count stale refs
        let mut stale_count = 0;
        for entry in self.index.vault_notes.iter() {
            for file_ref in &entry.value().file_refs {
                if self.index.resolve_file_ref(file_ref).is_empty() {
                    stale_count += 1;
                }
            }
        }

        let coverage_pct = if total_notes > 0 {
            (linked_notes as f64 / total_notes as f64 * 100.0) as u32
        } else {
            0
        };

        let mut report = format!(
            "## Doctrack Coverage Report\n\n\
            | Metric | Value |\n\
            |---|---|\n\
            | Vault notes | {} |\n\
            | Code files indexed | {} |\n\
            | Sym↔Doc links (verified) | {} |\n\
            | Sym↔Doc links (fuzzy) | {} |\n\
            | Link coverage | {}% of notes |\n\
            | Stale references | {} |\n",
            total_notes, total_code_files, total_links, fuzzy_links, coverage_pct, stale_count
        );

        if !undocumented.is_empty() {
            report.push_str(&format!(
                "\n### Undocumented code files ({}):\n",
                undocumented.len()
            ));
            for (i, file) in undocumented.iter().take(20).enumerate() {
                report.push_str(&format!("{}. `{}`\n", i + 1, file));
            }
            if undocumented.len() > 20 {
                report.push_str(&format!("... and {} more\n", undocumented.len() - 20));
            }
        }

        report
    }

    /// Report all stale references across the vault.
    #[tool(
        name = "stale_report",
        description = "Get a full list of all stale file references and broken wikilinks across the vault. Use to identify documentation that needs updating."
    )]
    async fn stale_report(&self) -> String {
        let mut stale_files = Vec::new();
        let mut broken_wikilinks = Vec::new();

        for entry in self.index.vault_notes.iter() {
            let note = entry.value();

            for file_ref in &note.file_refs {
                if self.index.resolve_file_ref(file_ref).is_empty() {
                    stale_files.push(format!(
                        "- **{}**: `{}` not found",
                        note.title,
                        file_ref.path.display()
                    ));
                }
            }

            for link in &note.wikilinks {
                let linked_path = self.index.vault_root.join(format!("{link}.md"));
                if !linked_path.exists() {
                    let found = self
                        .index
                        .vault_notes
                        .iter()
                        .any(|e| e.value().title.eq_ignore_ascii_case(link));
                    if !found {
                        broken_wikilinks
                            .push(format!("- **{}**: [[{}]] not found", note.title, link));
                    }
                }
            }
        }

        let mut report = String::new();

        if stale_files.is_empty() && broken_wikilinks.is_empty() {
            return "No stale references or broken wikilinks found.".to_string();
        }

        if !stale_files.is_empty() {
            report.push_str(&format!(
                "### Stale file references ({})\n\n",
                stale_files.len()
            ));
            for s in &stale_files {
                report.push_str(s);
                report.push('\n');
            }
        }

        if !broken_wikilinks.is_empty() {
            report.push_str(&format!(
                "\n### Broken wikilinks ({})\n\n",
                broken_wikilinks.len()
            ));
            for w in &broken_wikilinks {
                report.push_str(w);
                report.push('\n');
            }
        }

        report
    }

    /// Generate a refresh plan for stale documentation.
    #[tool(
        name = "refresh_docs",
        description = "Scan the vault and generate a prioritized plan of documentation that needs updating. Compares note last_updated timestamps against code file modification times, detects symbol drift (renamed/added/removed), and identifies undocumented code. Returns an actionable list — use it to drive targeted doc updates. Idempotent: run again after updating to verify nothing remains stale."
    )]
    async fn refresh_docs(&self) -> String {
        #[derive(Debug)]
        struct StaleNote {
            note_path: String,
            note_title: String,
            note_type: String,
            reasons: Vec<String>,
            priority: u32, // higher = more urgent
        }

        let mut stale_notes: Vec<StaleNote> = Vec::new();
        let mut undocumented_files: Vec<String> = Vec::new();

        // Phase 1: Check each vault note for staleness
        for entry in self.index.vault_notes.iter() {
            let note = entry.value();
            let mut reasons = Vec::new();
            let mut priority = 0u32;

            let note_rel = note
                .path
                .strip_prefix(&self.index.vault_root)
                .unwrap_or(&note.path)
                .display()
                .to_string();

            // Parse note's last_updated date
            let note_updated = note
                .frontmatter
                .last_updated
                .as_ref()
                .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());

            // Check each referenced code file
            for file_ref in &note.file_refs {
                let resolved = self.index.resolve_file_ref(file_ref);

                if resolved.is_empty() {
                    reasons.push(format!(
                        "Broken ref: `{}` not found",
                        file_ref.path.display()
                    ));
                    priority += 3;
                    continue;
                }

                for abs_path in &resolved {
                    // Compare file mtime against note last_updated
                    if let (Some(note_date), Ok(metadata)) =
                        (note_updated, std::fs::metadata(abs_path))
                        && let Ok(modified) = metadata.modified()
                    {
                        let file_date =
                            chrono::DateTime::<chrono::Utc>::from(modified).date_naive();
                        if file_date > note_date {
                            let days_stale = (file_date - note_date).num_days();
                            let rel = abs_path
                                .strip_prefix(&self.index.root)
                                .unwrap_or(abs_path)
                                .display();
                            reasons.push(format!(
                                "Code newer: `{}` modified {} day(s) after note was last updated",
                                rel, days_stale
                            ));
                            priority += if days_stale > 30 {
                                3
                            } else if days_stale > 7 {
                                2
                            } else {
                                1
                            };
                        }
                    }

                    // Check for symbol drift — symbols in code that the note might be missing
                    if let Some(symbols) = self.index.code_symbols.get(abs_path) {
                        let mut missing_symbols = Vec::new();
                        for sym in symbols.value() {
                            // If a symbol exists in the code but isn't mentioned in
                            // the note body. Identifier boundaries matter here: a
                            // substring test reports `new` as documented by the word
                            // "renewal" and `Index` by the word "index".
                            if !dt_index::vault::mentions_identifier(&note.body, &sym.name) {
                                missing_symbols.push(format!("`{}`", sym.name));
                            }
                        }
                        if !missing_symbols.is_empty() && missing_symbols.len() <= 10 {
                            let rel = abs_path
                                .strip_prefix(&self.index.root)
                                .unwrap_or(abs_path)
                                .display();
                            reasons.push(format!(
                                "Undocumented symbols in `{}`: {}",
                                rel,
                                missing_symbols.join(", ")
                            ));
                            priority += 1;
                        }
                    }
                }
            }

            // Check for broken wikilinks
            for link in &note.wikilinks {
                let linked_path = self.index.vault_root.join(format!("{link}.md"));
                if !linked_path.exists() {
                    let found = self
                        .index
                        .vault_notes
                        .iter()
                        .any(|e| e.value().title.eq_ignore_ascii_case(link));
                    if !found {
                        reasons.push(format!("Broken wikilink: [[{link}]]"));
                        priority += 1;
                    }
                }
            }

            // Check if note has no last_updated at all
            if note.frontmatter.last_updated.is_none() && !note.file_refs.is_empty() {
                reasons
                    .push("Missing `last_updated` frontmatter — can't track staleness".to_string());
                priority += 1;
            }

            if !reasons.is_empty() {
                stale_notes.push(StaleNote {
                    note_path: note_rel,
                    note_title: note.title.clone(),
                    note_type: note.note_type.clone(),
                    reasons,
                    priority,
                });
            }
        }

        // Phase 2: Find code files with no documentation at all
        for entry in self.index.code_symbols.iter() {
            let file = entry.key();
            let has_docs = entry.value().iter().any(|sym| {
                !self
                    .index
                    .verified_docs_for_symbol(file, &sym.name)
                    .is_empty()
            });
            if !has_docs {
                let rel = file
                    .strip_prefix(&self.index.root)
                    .unwrap_or(file)
                    .display()
                    .to_string();
                undocumented_files.push(rel);
            }
        }
        undocumented_files.sort();

        // Phase 3: Find new docs/READMEs not yet imported into the vault
        let mut new_docs: Vec<String> = Vec::new();
        {
            // Collect all original_path and file refs from vault notes to know what's imported
            let mut imported_paths = std::collections::HashSet::new();
            for entry in self.index.vault_notes.iter() {
                let note = entry.value();
                // Check frontmatter for original_path or files
                for file_ref in &note.file_refs {
                    imported_paths.insert(file_ref.path.to_string_lossy().to_string());
                }
                // Also check body for references to doc files
                for line in note.body.lines() {
                    let trimmed = line.trim();
                    if let Some(path) = trimmed.strip_prefix("original_path:") {
                        imported_paths.insert(path.trim().to_string());
                    }
                }
            }

            let skip_dirs = [
                "target",
                "node_modules",
                ".git",
                ".doctrack",
                ".idea",
                ".vscode",
                "build",
                "dist",
                "out",
                "__pycache__",
                ".gradle",
                "vendor",
                ".next",
                ".claude",
                ".agents",
            ];

            for entry in walkdir::WalkDir::new(&self.index.root)
                .into_iter()
                .filter_entry(|e| {
                    let name = e.file_name().to_string_lossy();
                    if e.file_type().is_dir() {
                        return !skip_dirs.contains(&name.as_ref()) && !name.starts_with('.');
                    }
                    true
                })
                .filter_map(|e| e.ok())
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();

                // Only look for markdown docs, READMEs, and doc directories
                let is_doc = ext == "md"
                    || filename.eq_ignore_ascii_case("README")
                    || filename.eq_ignore_ascii_case("CHANGELOG")
                    || filename.eq_ignore_ascii_case("CONTRIBUTING");

                if !is_doc {
                    continue;
                }

                let rel = path
                    .strip_prefix(&self.index.root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                // Skip if already imported
                if imported_paths.contains(&rel) || imported_paths.contains(&filename) {
                    continue;
                }

                // Skip if there's a vault note with a matching title
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let already_in_vault = self.index.vault_notes.iter().any(|e| {
                    let title = &e.value().title;
                    title.eq_ignore_ascii_case(&stem) || title.eq_ignore_ascii_case(&filename)
                });
                if already_in_vault {
                    continue;
                }

                new_docs.push(rel);
            }
            new_docs.sort();
        }

        // Build the report
        stale_notes.sort_by(|a, b| b.priority.cmp(&a.priority));

        if stale_notes.is_empty() && undocumented_files.is_empty() && new_docs.is_empty() {
            return "All documentation is up to date. No refresh needed.".to_string();
        }

        let mut report = String::from("## Documentation Refresh Plan\n\n");

        if !stale_notes.is_empty() {
            report.push_str(&format!(
                "### Stale notes ({} need updating)\n\n",
                stale_notes.len()
            ));

            for (i, note) in stale_notes.iter().enumerate() {
                let priority_label = match note.priority {
                    0..=2 => "LOW",
                    3..=4 => "MEDIUM",
                    _ => "HIGH",
                };
                report.push_str(&format!(
                    "**{}. {} [{}]** — `{}` (priority: {})\n",
                    i + 1,
                    note.note_title,
                    note.note_type,
                    note.note_path,
                    priority_label
                ));
                for reason in &note.reasons {
                    report.push_str(&format!("   - {reason}\n"));
                }
                report.push('\n');
            }
        }

        if !undocumented_files.is_empty() {
            report.push_str(&format!(
                "### Undocumented code files ({})\n\n",
                undocumented_files.len()
            ));
            for (i, file) in undocumented_files.iter().take(20).enumerate() {
                report.push_str(&format!("{}. `{}`\n", i + 1, file));
            }
            if undocumented_files.len() > 20 {
                report.push_str(&format!("... and {} more\n", undocumented_files.len() - 20));
            }
        }

        if !new_docs.is_empty() {
            report.push_str(&format!(
                "### New docs not yet imported ({})\n\nThese markdown/README files exist in the project but aren't in the vault. Import them to `references/imported/`.\n\n",
                new_docs.len()
            ));
            for (i, file) in new_docs.iter().take(20).enumerate() {
                report.push_str(&format!("{}. `{}`\n", i + 1, file));
            }
            if new_docs.len() > 20 {
                report.push_str(&format!("... and {} more\n", new_docs.len() - 20));
            }
        }

        report.push_str(
            "\n---\n*Run `refresh_docs` again after updating to verify all issues are resolved.*",
        );

        report
    }

    /// Search the index for notes, symbols, or files matching a query.
    #[tool(
        name = "search_index",
        description = "Fuzzy search across vault notes, code symbols, and file paths. Use when you need to find related documentation or code by keyword."
    )]
    async fn search_index(&self, Parameters(input): Parameters<SearchIndexInput>) -> String {
        use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
        use nucleo_matcher::{Config, Matcher};

        let mut matcher = Matcher::new(Config::DEFAULT);

        // Split query into tokens for multi-word matching
        let tokens: Vec<&str> = input.query.split_whitespace().collect();
        let patterns: Vec<Pattern> = tokens
            .iter()
            .map(|t| Pattern::parse(t, CaseMatching::Ignore, Normalization::Smart))
            .collect();

        // Also create a combined pattern for single-token matching
        let combined = Pattern::parse(&input.query, CaseMatching::Ignore, Normalization::Smart);

        let mut results = Vec::new();

        // Search vault note titles AND summaries
        for entry in self.index.vault_notes.iter() {
            let note = entry.value();
            // Build a searchable string from title + summary + tags
            let searchable = format!(
                "{} {} {}",
                note.title,
                note.summary,
                note.frontmatter.tags.join(" ")
            );

            let score = multi_token_score(&searchable, &patterns, &combined, &mut matcher);
            if score > 0 {
                results.push((
                    score,
                    format!(
                        "- **Note**: {} [{}] (score: {})",
                        note.title, note.note_type, score
                    ),
                ));
            }
        }

        // Search code symbols
        for entry in self.index.code_symbols.iter() {
            let file = entry.key();
            let rel = file.strip_prefix(&self.index.root).unwrap_or(file);
            for sym in entry.value() {
                let searchable = format!("{} {}", sym.name, rel.display());
                let score = multi_token_score(&searchable, &patterns, &combined, &mut matcher);
                if score > 0 {
                    results.push((
                        score,
                        format!(
                            "- **Symbol**: {} `{}` in `{}` L{} (score: {})",
                            sym.kind,
                            sym.name,
                            rel.display(),
                            sym.start_line + 1,
                            score
                        ),
                    ));
                }
            }
        }

        results.sort_by(|a, b| b.0.cmp(&a.0));
        results.truncate(20);

        if results.is_empty() {
            format!("No results found for `{}`", input.query)
        } else {
            let lines: Vec<_> = results.iter().map(|(_, line)| line.as_str()).collect();
            format!(
                "Search results for `{}`:\n\n{}",
                input.query,
                lines.join("\n")
            )
        }
    }
}

/// Score a haystack against multiple token patterns.
/// Returns the sum of individual token scores if ALL tokens match,
/// or the combined pattern score, whichever is higher.
fn multi_token_score(
    haystack: &str,
    token_patterns: &[nucleo_matcher::pattern::Pattern],
    combined: &nucleo_matcher::pattern::Pattern,
    matcher: &mut nucleo_matcher::Matcher,
) -> u32 {
    let threshold = 20u32;

    // Try combined pattern first
    let mut buf = Vec::new();
    let combined_score = combined
        .score(nucleo_matcher::Utf32Str::new(haystack, &mut buf), matcher)
        .unwrap_or(0);

    // If single token query, just use combined
    if token_patterns.len() <= 1 {
        return if combined_score > threshold {
            combined_score
        } else {
            0
        };
    }

    // For multi-token: each token must match, sum their scores
    let mut total = 0u32;
    let mut all_matched = true;
    for pattern in token_patterns {
        let mut buf = Vec::new();
        if let Some(score) =
            pattern.score(nucleo_matcher::Utf32Str::new(haystack, &mut buf), matcher)
        {
            if score > threshold {
                total = total.saturating_add(score);
            } else {
                all_matched = false;
                break;
            }
        } else {
            all_matched = false;
            break;
        }
    }

    let multi_score = if all_matched { total } else { 0 };

    // Return the better of the two approaches
    multi_score.max(if combined_score > threshold {
        combined_score
    } else {
        0
    })
}
