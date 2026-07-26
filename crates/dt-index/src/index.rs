use std::path::{Path, PathBuf};

use anyhow::Result;
use dashmap::DashMap;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::symbols::CodeSymbol;
use crate::vault::VaultNote;

/// Upper bound on source files parsed during a full index build. Large enough
/// for any repo doctrack is realistically pointed at; low enough that a stray
/// `.doctrack/` in a home directory doesn't parse the whole disk.
pub const DEFAULT_MAX_INDEXED_FILES: usize = 20_000;

fn max_indexed_files() -> usize {
    std::env::var("DOCTRACK_MAX_INDEXED_FILES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_INDEXED_FILES)
}

/// Unique identifier for a code symbol.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SymbolId {
    pub file: PathBuf,
    pub name: String,
}

/// A link from a vault note to a code symbol, with surrounding context.
#[derive(Debug, Clone)]
pub struct DocLink {
    pub note_path: PathBuf,
    pub note_title: String,
    pub note_type: String,
    pub context: String,
    /// How the link was established. Consumers must not present a `Fuzzy` link
    /// as documented fact.
    pub confidence: MatchConfidence,
}

/// A reference from a vault note to a code location.
#[derive(Debug, Clone)]
pub struct SymbolRef {
    pub symbol_id: SymbolId,
    pub confidence: MatchConfidence,
}

/// Ordered strongest-first: `Exact < Strong < Fuzzy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchConfidence {
    /// Explicit file path in frontmatter file-registry
    Exact,
    /// Symbol name found in backticks matching a parsed symbol
    Strong,
    /// Fuzzy title/filename match
    Fuzzy,
}

impl MatchConfidence {
    /// True for links traceable to something the note actually says.
    ///
    /// `Fuzzy` links are guesses from title similarity — useful as navigation
    /// hints, but they must not count toward coverage or be reported as
    /// documentation.
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Exact | Self::Strong)
    }
}

impl std::fmt::Display for MatchConfidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exact => write!(f, "exact"),
            Self::Strong => write!(f, "strong"),
            Self::Fuzzy => write!(f, "fuzzy"),
        }
    }
}

/// The bidirectional code↔documentation index.
pub struct Index {
    /// Code file path → symbols extracted from it
    pub code_symbols: DashMap<PathBuf, Vec<CodeSymbol>>,
    /// Vault note path → parsed note metadata
    pub vault_notes: DashMap<PathBuf, VaultNote>,
    /// Symbol → notes that reference it
    pub sym_to_docs: DashMap<SymbolId, Vec<DocLink>>,
    /// Note → symbols it references
    pub doc_to_syms: DashMap<PathBuf, Vec<SymbolRef>>,
    /// Filename → list of absolute paths (for bare filename resolution)
    pub file_lookup: DashMap<String, Vec<PathBuf>>,
    /// Symbol name → every place it is defined. Without this, resolving a
    /// backtick identifier means scanning every symbol in the codebase.
    pub symbol_names: DashMap<String, Vec<SymbolId>>,
    /// Project root (for resolving relative paths)
    pub root: PathBuf,
    /// Vault root (.doctrack/ directory)
    pub vault_root: PathBuf,
}

impl Index {
    pub fn new(root: PathBuf, vault_root: PathBuf) -> Self {
        Self {
            code_symbols: DashMap::new(),
            vault_notes: DashMap::new(),
            sym_to_docs: DashMap::new(),
            doc_to_syms: DashMap::new(),
            file_lookup: DashMap::new(),
            symbol_names: DashMap::new(),
            root,
            vault_root,
        }
    }

    /// Full index build — parse all vault notes + code symbols, then link them.
    pub fn build(&self) -> Result<()> {
        info!("building index from vault: {:?}", self.vault_root);

        // Phase 1: Build the project file lookup table
        self.build_file_lookup();
        info!("file lookup: {} unique filenames", self.file_lookup.len());

        // Phase 2: Parse all vault notes
        let notes = crate::vault::parse_vault(&self.vault_root)?;
        for note in &notes {
            self.vault_notes.insert(note.path.clone(), note.clone());
        }
        info!("indexed {} vault notes", self.vault_notes.len());

        // Phase 3: Extract code symbols.
        //
        // Every parseable source file in the project is indexed, not only the
        // ones notes already point at. Indexing just the referenced set makes
        // "which files are undocumented?" unanswerable by construction, and
        // leaves symbol lookups blind to anything not yet written up.
        let code_files = self.collect_code_files();
        let mut skipped = 0usize;
        for file in &code_files {
            match crate::symbols::extract_symbols(file) {
                Ok(symbols) => {
                    self.set_symbols(file, symbols);
                }
                Err(e) => {
                    skipped += 1;
                    debug!("skipping {}: {}", file.display(), e);
                }
            }
        }
        info!(
            "indexed symbols from {} code files ({skipped} skipped)",
            self.code_symbols.len()
        );

        // Phase 4: Build bidirectional links
        crate::matching::link_all(self);
        info!(
            "linked {} symbol→doc mappings, {} doc→symbol mappings",
            self.sym_to_docs.len(),
            self.doc_to_syms.len()
        );

        Ok(())
    }

    /// Walk the project tree and build a filename → [absolute paths] lookup.
    fn build_file_lookup(&self) {
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
        ];

        for entry in WalkDir::new(&self.root)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                // Skip hidden dirs (except the ones we explicitly handle) and known junk
                if e.file_type().is_dir() {
                    return !skip_dirs.contains(&name.as_ref()) && !name.starts_with('.');
                }
                true
            })
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                let path = entry.path();
                if let Some(filename) = path.file_name() {
                    let name = filename.to_string_lossy().to_string();
                    self.file_lookup
                        .entry(name)
                        .or_default()
                        .push(path.to_path_buf());
                }
            }
        }
    }

    /// Resolve a file reference to an absolute path.
    /// Handles relative paths, bare filenames, and abbreviated `...` paths.
    pub fn resolve_file_ref(&self, file_ref: &crate::vault::FileRef) -> Vec<PathBuf> {
        let path_str = file_ref.path.to_string_lossy();

        // Handle paths with ... abbreviation (e.g. "ci-reporting/src/main/java/.../config/Foo.java")
        if path_str.contains("/...") || path_str.contains(".../") {
            return self.resolve_abbreviated_path(&path_str);
        }

        if file_ref.is_bare_filename {
            // Bare filename like "CertificateInfo.java" — search the lookup table
            if let Some(paths) = self.file_lookup.get(&*path_str) {
                return paths.value().clone();
            }
            // Try with just the filename component in case path has a shallow prefix
            if let Some(filename) = file_ref.path.file_name() {
                let name = filename.to_string_lossy().to_string();
                if let Some(paths) = self.file_lookup.get(&name) {
                    return paths.value().clone();
                }
            }
            vec![]
        } else {
            // Relative or absolute path — resolve against project root
            let abs = self.root.join(&file_ref.path);
            if abs.exists() {
                vec![abs]
            } else {
                // Maybe the path is partial (e.g. "dto/ListenerInfoDto.java").
                // The filename is the one component a partial path always ends
                // with, so start from the lookup table instead of scanning it.
                let Some(filename) = file_ref.path.file_name() else {
                    return vec![];
                };
                let name = filename.to_string_lossy().to_string();
                self.file_lookup
                    .get(&name)
                    .map(|candidates| {
                        candidates
                            .value()
                            .iter()
                            .filter(|full| crate::vault::ref_matches_path(&file_ref.path, full))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default()
            }
        }
    }

    /// Resolve an abbreviated path containing `...` as a wildcard.
    /// e.g. "ci-reporting/src/main/java/.../config/ReportingProperties.java"
    /// matches any file where the prefix and suffix segments align.
    fn resolve_abbreviated_path(&self, path_str: &str) -> Vec<PathBuf> {
        // Split on ... to get prefix and suffix segments
        let parts: Vec<&str> = path_str.split("...").collect();
        if parts.len() != 2 {
            return vec![];
        }

        let prefix = parts[0].trim_end_matches('/');
        let suffix = parts[1].trim_start_matches('/');

        let mut matches = Vec::new();

        for entry in self.file_lookup.iter() {
            for full_path in entry.value() {
                let rel = full_path
                    .strip_prefix(&self.root)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();

                // Both prefix and suffix must match
                if rel.starts_with(prefix) && rel.ends_with(suffix) {
                    matches.push(full_path.clone());
                }
            }
        }

        matches
    }

    /// Collect every parseable source file in the project, plus anything the
    /// vault references that the project walk didn't reach.
    ///
    /// Bounded by `DOCTRACK_MAX_INDEXED_FILES` (default
    /// [`DEFAULT_MAX_INDEXED_FILES`]). Files referenced by notes are kept ahead
    /// of the rest so a cap never costs us a documented file, and a cap that
    /// bites is logged rather than silently truncating the index.
    fn collect_code_files(&self) -> Vec<PathBuf> {
        let mut referenced = std::collections::HashSet::new();
        for entry in self.vault_notes.iter() {
            for file_ref in &entry.value().file_refs {
                for resolved in self.resolve_file_ref(file_ref) {
                    if crate::symbols::is_supported(&resolved) {
                        referenced.insert(resolved);
                    }
                }
            }
        }
        debug!(
            "resolved {} unique code files from vault references",
            referenced.len()
        );

        let mut rest: Vec<PathBuf> = self
            .file_lookup
            .iter()
            .flat_map(|e| e.value().clone())
            .filter(|p| crate::symbols::is_supported(p) && !referenced.contains(p))
            .collect();
        rest.sort();

        let limit = max_indexed_files();
        let mut files: Vec<PathBuf> = referenced.into_iter().collect();
        files.sort();

        if files.len() + rest.len() > limit {
            let room = limit.saturating_sub(files.len());
            let dropped = rest.len() - room.min(rest.len());
            warn!(
                "project has more source files than DOCTRACK_MAX_INDEXED_FILES ({limit}); \
                 skipping {dropped} unreferenced file(s) — symbol lookups and coverage \
                 will be incomplete for those"
            );
            rest.truncate(room);
        }

        files.extend(rest);
        files
    }

    /// Replace the symbols recorded for a code file, keeping `symbol_names` and
    /// the symbol→doc map consistent with the new symbol set.
    pub fn set_symbols(&self, file: &Path, symbols: Vec<CodeSymbol>) {
        let new_names: std::collections::HashSet<&str> =
            symbols.iter().map(|s| s.name.as_str()).collect();

        // Retire names that this file no longer defines.
        if let Some(previous) = self.code_symbols.get(file) {
            let gone: Vec<String> = previous
                .value()
                .iter()
                .filter(|s| !new_names.contains(s.name.as_str()))
                .map(|s| s.name.clone())
                .collect();
            drop(previous);

            for name in gone {
                let id = SymbolId {
                    file: file.to_path_buf(),
                    name: name.clone(),
                };
                self.sym_to_docs.remove(&id);
                if let Some(mut ids) = self.symbol_names.get_mut(&name) {
                    ids.retain(|existing| existing != &id);
                }
                self.symbol_names.remove_if(&name, |_, ids| ids.is_empty());
            }
        }

        for sym in &symbols {
            let id = SymbolId {
                file: file.to_path_buf(),
                name: sym.name.clone(),
            };
            let mut ids = self.symbol_names.entry(sym.name.clone()).or_default();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }

        self.code_symbols.insert(file.to_path_buf(), symbols);
    }

    /// Drop every link originating from a note, leaving no dangling `DocLink`s
    /// in `sym_to_docs`.
    pub fn unlink_note(&self, note_path: &Path) {
        let Some((_, refs)) = self.doc_to_syms.remove(note_path) else {
            return;
        };

        let mut emptied = Vec::new();
        for r in &refs {
            if let Some(mut docs) = self.sym_to_docs.get_mut(&r.symbol_id) {
                docs.retain(|d| d.note_path != note_path);
                if docs.is_empty() {
                    emptied.push(r.symbol_id.clone());
                }
            }
        }
        // Removing inside the `get_mut` guard above would deadlock the shard.
        for id in emptied {
            self.sym_to_docs.remove_if(&id, |_, docs| docs.is_empty());
        }
    }

    /// Re-index a single vault note (on file change), rebuilding its links.
    pub fn reindex_note(&self, path: &Path) -> Result<()> {
        let note = crate::vault::parse_note(path)?;
        self.unlink_note(path);
        self.vault_notes.insert(path.to_path_buf(), note.clone());
        crate::matching::link_note(self, &note);
        Ok(())
    }

    /// Forget a vault note entirely (on delete).
    pub fn remove_note(&self, path: &Path) {
        self.unlink_note(path);
        self.vault_notes.remove(path);
    }

    /// Re-index a single code file (on file change), rebuilding the links of
    /// every note that could be affected by its new symbol set.
    pub fn reindex_code_file(&self, path: &Path) -> Result<()> {
        let symbols = crate::symbols::extract_symbols(path)?;
        self.set_symbols(path, symbols);
        self.relink_notes_for_file(path);
        Ok(())
    }

    /// Forget a code file entirely (on delete).
    pub fn remove_code_file(&self, path: &Path) {
        self.set_symbols(path, Vec::new());
        self.code_symbols.remove(path);
        self.relink_notes_for_file(path);
    }

    /// Recompute links for the notes that reference `path`, either by file
    /// reference or by naming one of the symbols it defines.
    fn relink_notes_for_file(&self, path: &Path) {
        let names: std::collections::HashSet<String> = self
            .code_symbols
            .get(path)
            .map(|s| s.value().iter().map(|sym| sym.name.clone()).collect())
            .unwrap_or_default();

        let mut affected: std::collections::HashSet<PathBuf> = self
            .doc_to_syms
            .iter()
            .filter(|e| e.value().iter().any(|r| r.symbol_id.file == path))
            .map(|e| e.key().clone())
            .collect();

        for entry in self.vault_notes.iter() {
            let note = entry.value();
            if affected.contains(&note.path) {
                continue;
            }
            let names_it = note.code_idents.iter().any(|i| names.contains(i));
            let refs_it = note
                .file_refs
                .iter()
                .any(|fr| self.resolve_file_ref(fr).iter().any(|p| p == path));
            if names_it || refs_it {
                affected.insert(note.path.clone());
            }
        }

        for note_path in affected {
            let note = self.vault_notes.get(&note_path).map(|n| n.value().clone());
            self.unlink_note(&note_path);
            if let Some(note) = note {
                crate::matching::link_note(self, &note);
            }
        }
    }

    /// Look up all documentation links for a given symbol.
    pub fn docs_for_symbol(&self, file: &Path, name: &str) -> Vec<DocLink> {
        let id = SymbolId {
            file: file.to_path_buf(),
            name: name.to_string(),
        };
        self.sym_to_docs
            .get(&id)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// Documentation links for a symbol, excluding fuzzy title guesses.
    pub fn verified_docs_for_symbol(&self, file: &Path, name: &str) -> Vec<DocLink> {
        let mut docs = self.docs_for_symbol(file, name);
        docs.retain(|d| d.confidence.is_verified());
        docs
    }

    /// Every definition site recorded for a symbol name.
    pub fn definitions_of(&self, name: &str) -> Vec<SymbolId> {
        self.symbol_names
            .get(name)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// Look up all symbol references for a given vault note.
    pub fn symbols_for_note(&self, note_path: &Path) -> Vec<SymbolRef> {
        self.doc_to_syms
            .get(note_path)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }
}
