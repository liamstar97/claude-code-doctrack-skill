//! End-to-end tests over a real on-disk project + vault.
//!
//! The unit tests cover parsing helpers in isolation; these cover the part that
//! was previously untested and where the interesting bugs lived: building the
//! index, resolving references, linking, and keeping links correct as files
//! change underneath it.

use std::fs;
use std::path::{Path, PathBuf};

use dt_index::index::{Index, MatchConfidence};

/// A throwaway project directory containing a `.doctrack/` vault.
struct Fixture {
    root: PathBuf,
    _guard: TempDir,
}

/// Minimal scoped temp directory — avoids pulling in a dev-dependency for four
/// lines of behaviour.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "doctrack-test-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".doctrack")).unwrap();
        Self {
            _guard: TempDir(root.clone()),
            root,
        }
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    fn note(&self, rel: &str, contents: &str) -> PathBuf {
        self.write(&format!(".doctrack/{rel}"), contents)
    }

    fn index(&self) -> Index {
        let index = Index::new(self.root.clone(), self.root.join(".doctrack"));
        index.build().unwrap();
        index
    }
}

fn confidences(index: &Index, note: &Path) -> Vec<MatchConfidence> {
    let mut c: Vec<_> = index
        .symbols_for_note(note)
        .into_iter()
        .map(|r| r.confidence)
        .collect();
    c.sort();
    c.dedup();
    c
}

fn linked_symbol_names(index: &Index, note: &Path) -> Vec<String> {
    let mut names: Vec<_> = index
        .symbols_for_note(note)
        .into_iter()
        .map(|r| r.symbol_id.name)
        .collect();
    names.sort();
    names
}

const SESSION_RS: &str = "\
pub struct SessionManager {
    id: u32,
}

pub fn authenticate(user: &str) -> bool {
    !user.is_empty()
}
";

#[test]
fn builds_index_and_links_frontmatter_refs_exactly() {
    let fx = Fixture::new("exact");
    fx.write("src/auth/session.rs", SESSION_RS);
    let note = fx.note(
        "features/auth.md",
        "---\ntype: feature\nfiles:\n  - src/auth/session.rs\n---\n\n# Authentication\n\nHandles login.\n",
    );

    let index = fx.index();

    assert_eq!(index.vault_notes.len(), 1);
    assert_eq!(
        index.code_symbols.len(),
        1,
        "the referenced file is indexed"
    );
    assert_eq!(
        linked_symbol_names(&index, &note),
        vec!["SessionManager", "authenticate"]
    );
    assert_eq!(confidences(&index, &note), vec![MatchConfidence::Exact]);

    let docs = index.docs_for_symbol(&fx.root.join("src/auth/session.rs"), "authenticate");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].note_title, "Authentication");
    assert!(docs[0].confidence.is_verified());
}

#[test]
fn a_file_referenced_twice_produces_one_link() {
    // Regression: tier 1 pushed a DocLink per (file_ref, symbol) pair, so a file
    // named in frontmatter *and* mentioned in prose was counted twice.
    let fx = Fixture::new("dupes");
    fx.write("src/auth/session.rs", SESSION_RS);
    fx.note(
        "features/auth.md",
        "---\ntype: feature\nfiles:\n  - src/auth/session.rs\n---\n\n# Auth\n\nSee `src/auth/session.rs` for details.\n",
    );

    let index = fx.index();
    let docs = index.docs_for_symbol(&fx.root.join("src/auth/session.rs"), "SessionManager");
    assert_eq!(docs.len(), 1, "expected one link, got {docs:#?}");
}

#[test]
fn backtick_identifiers_link_strongly_and_fenced_blocks_do_not() {
    let fx = Fixture::new("strong");
    fx.write("src/auth/session.rs", SESSION_RS);
    fx.write("src/other.rs", "pub fn phantom() {}\n");

    // `other.rs` is only reachable because a note references it by path;
    // `phantom` is named only inside a fenced block and must not link.
    let note = fx.note(
        "components/session.md",
        "---\ntype: component\nfiles:\n  - src/other.rs\n---\n\n# Session\n\n\
         Delegates to `SessionManager`.\n\n\
         ```rust\nlet x = phantom();\n```\n",
    );

    let index = fx.index();
    let names = linked_symbol_names(&index, &note);
    assert!(names.contains(&"SessionManager".to_string()));
    assert!(
        names.contains(&"phantom".to_string()),
        "phantom is linked via the file reference, not the fenced block"
    );

    let phantom_docs = index.docs_for_symbol(&fx.root.join("src/other.rs"), "phantom");
    assert_eq!(phantom_docs.len(), 1);
    assert_eq!(
        phantom_docs[0].confidence,
        MatchConfidence::Exact,
        "the link came from the file reference; the fenced mention added nothing"
    );

    let session_docs =
        index.docs_for_symbol(&fx.root.join("src/auth/session.rs"), "SessionManager");
    assert_eq!(session_docs[0].confidence, MatchConfidence::Strong);
}

#[test]
fn project_file_registry_table_is_indexed() {
    // The `_project.md` convention keeps the registry in a markdown table.
    let fx = Fixture::new("registry");
    fx.write("src/auth/session.rs", SESSION_RS);
    let note = fx.note(
        "_project.md",
        "---\ntype: index\n---\n\n# Demo\n\n## File Registry\n\n\
         | Source File | Feature | Component |\n\
         |------------|---------|-----------|\n\
         | src/auth/session.rs | auth | session |\n",
    );

    let index = fx.index();
    assert_eq!(
        linked_symbol_names(&index, &note),
        vec!["SessionManager", "authenticate"],
        "the registry table should be a source of file references"
    );
}

#[test]
fn partial_paths_resolve_without_matching_lookalike_files() {
    let fx = Fixture::new("partial");
    fx.write("src/auth/session.rs", SESSION_RS);
    fx.write("src/auth/mysession.rs", "pub fn decoy() {}\n");
    let note = fx.note(
        "features/auth.md",
        "---\ntype: feature\nfiles:\n  - auth/session.rs\n---\n\n# Auth\n",
    );

    let index = fx.index();
    let names = linked_symbol_names(&index, &note);
    assert!(names.contains(&"SessionManager".to_string()));
    assert!(
        !names.contains(&"decoy".to_string()),
        "`auth/session.rs` must not resolve to `auth/mysession.rs`"
    );
}

#[test]
fn stale_reference_resolves_to_nothing() {
    let fx = Fixture::new("stale");
    fx.write("src/auth/session.rs", SESSION_RS);
    let note_path = fx.note(
        "features/auth.md",
        "---\ntype: feature\nfiles:\n  - src/auth/deleted.rs\n---\n\n# Auth\n",
    );

    let index = fx.index();
    let note = index.vault_notes.get(&note_path).unwrap();
    assert_eq!(note.file_refs.len(), 1);
    assert!(index.resolve_file_ref(&note.file_refs[0]).is_empty());
}

#[test]
fn reindexing_a_note_replaces_its_links() {
    // Regression: reindex_note used to swap the parsed note in place while
    // leaving both link maps untouched, so edits never took effect and stale
    // DocLinks accumulated forever.
    let fx = Fixture::new("reindex-note");
    fx.write("src/a.rs", "pub fn alpha() {}\n");
    fx.write("src/b.rs", "pub fn beta() {}\n");
    let note = fx.note(
        "features/x.md",
        "---\ntype: feature\nfiles:\n  - src/a.rs\n  - src/b.rs\n---\n\n# X\n",
    );

    let index = fx.index();
    assert_eq!(linked_symbol_names(&index, &note), vec!["alpha", "beta"]);

    // Drop the reference to b.rs.
    fs::write(
        &note,
        "---\ntype: feature\nfiles:\n  - src/a.rs\n---\n\n# X\n",
    )
    .unwrap();
    index.reindex_note(&note).unwrap();

    assert_eq!(linked_symbol_names(&index, &note), vec!["alpha"]);
    assert!(
        index
            .docs_for_symbol(&fx.root.join("src/b.rs"), "beta")
            .is_empty(),
        "the dropped reference must not leave a dangling DocLink"
    );
}

#[test]
fn deleting_a_note_clears_its_links() {
    let fx = Fixture::new("remove-note");
    fx.write("src/a.rs", "pub fn alpha() {}\n");
    let note = fx.note(
        "features/x.md",
        "---\ntype: feature\nfiles:\n  - src/a.rs\n---\n\n# X\n",
    );

    let index = fx.index();
    assert_eq!(
        index
            .docs_for_symbol(&fx.root.join("src/a.rs"), "alpha")
            .len(),
        1
    );

    index.remove_note(&note);

    assert!(index.vault_notes.get(&note).is_none());
    assert!(index.symbols_for_note(&note).is_empty());
    assert!(
        index
            .docs_for_symbol(&fx.root.join("src/a.rs"), "alpha")
            .is_empty()
    );
}

#[test]
fn renaming_a_symbol_updates_links_in_both_directions() {
    // Regression: reindex_code_file replaced the symbol list but left
    // sym_to_docs keyed on the old name, so a renamed function stayed
    // "documented" forever and the new one was invisible.
    let fx = Fixture::new("rename-symbol");
    let code = fx.write("src/a.rs", "pub fn old_name() {}\n");
    let note = fx.note(
        "features/x.md",
        "---\ntype: feature\nfiles:\n  - src/a.rs\n---\n\n# X\n",
    );

    let index = fx.index();
    assert_eq!(index.docs_for_symbol(&code, "old_name").len(), 1);
    assert_eq!(index.definitions_of("old_name").len(), 1);

    fs::write(&code, "pub fn new_name() {}\n").unwrap();
    index.reindex_code_file(&code).unwrap();

    assert!(
        index.docs_for_symbol(&code, "old_name").is_empty(),
        "the removed symbol must not keep its documentation link"
    );
    assert!(index.definitions_of("old_name").is_empty());
    assert_eq!(index.docs_for_symbol(&code, "new_name").len(), 1);
    assert_eq!(linked_symbol_names(&index, &note), vec!["new_name"]);
}

#[test]
fn adding_a_symbol_named_in_prose_creates_a_link() {
    let fx = Fixture::new("add-symbol");
    let code = fx.write("src/a.rs", "pub fn alpha() {}\n");
    fx.write("src/anchor.rs", "pub fn anchor() {}\n");
    let note = fx.note(
        "features/x.md",
        "---\ntype: feature\nfiles:\n  - src/anchor.rs\n---\n\n# X\n\nWill call `gamma` soon.\n",
    );

    let index = fx.index();
    assert!(!linked_symbol_names(&index, &note).contains(&"gamma".to_string()));

    fs::write(&code, "pub fn alpha() {}\npub fn gamma() {}\n").unwrap();
    index.reindex_code_file(&code).unwrap();

    assert_eq!(index.docs_for_symbol(&code, "gamma").len(), 1);
    assert!(linked_symbol_names(&index, &note).contains(&"gamma".to_string()));
}

#[test]
fn removing_a_code_file_clears_its_links() {
    let fx = Fixture::new("remove-code");
    let code = fx.write("src/a.rs", "pub fn alpha() {}\n");
    let note = fx.note(
        "features/x.md",
        "---\ntype: feature\nfiles:\n  - src/a.rs\n---\n\n# X\n",
    );

    let index = fx.index();
    assert_eq!(index.docs_for_symbol(&code, "alpha").len(), 1);

    fs::remove_file(&code).unwrap();
    index.remove_code_file(&code);

    assert!(index.code_symbols.get(&code).is_none());
    assert!(index.definitions_of("alpha").is_empty());
    assert!(index.docs_for_symbol(&code, "alpha").is_empty());
    assert!(index.symbols_for_note(&note).is_empty());
}

#[test]
fn obsidian_internals_are_not_indexed_as_notes() {
    let fx = Fixture::new("obsidian");
    fx.write("src/a.rs", "pub fn alpha() {}\n");
    fx.note("features/x.md", "---\ntype: feature\n---\n\n# X\n");
    fx.note(".obsidian/plugins/some-plugin/README.md", "# Plugin docs\n");
    fx.note(".trash/deleted-note.md", "# Deleted\n");

    let index = fx.index();
    assert_eq!(
        index.vault_notes.len(),
        1,
        "only real notes should be indexed, found: {:#?}",
        index
            .vault_notes
            .iter()
            .map(|e| e.key().clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn unreferenced_source_files_are_indexed_and_reported_as_undocumented() {
    // Regression: only files a note already pointed at were parsed, which made
    // "which files lack documentation?" unanswerable by construction and left
    // symbol lookups blind to anything not yet written up.
    let fx = Fixture::new("whole-project");
    fx.write("src/documented.rs", "pub fn documented_fn() {}\n");
    fx.write("src/forgotten.rs", "pub fn forgotten_fn() {}\n");
    fx.note(
        "features/x.md",
        "---\ntype: feature\nfiles:\n  - src/documented.rs\n---\n\n# X\n",
    );

    let index = fx.index();

    assert_eq!(index.code_symbols.len(), 2, "both files should be parsed");
    assert_eq!(index.definitions_of("forgotten_fn").len(), 1);
    assert!(
        index
            .verified_docs_for_symbol(&fx.root.join("src/forgotten.rs"), "forgotten_fn")
            .is_empty(),
        "an unreferenced file should be indexed but undocumented"
    );
}

#[test]
fn an_ambiguous_identifier_does_not_link_to_everything() {
    // A note mentioning `run` shouldn't attach itself to every `run` in the repo.
    let fx = Fixture::new("fanout");
    for i in 0..12 {
        fx.write(&format!("src/m{i}.rs"), "pub fn run() {}\n");
    }
    fx.write("src/anchor.rs", "pub fn anchor() {}\n");
    let note = fx.note(
        "features/x.md",
        "---\ntype: feature\nfiles:\n  - src/anchor.rs\n---\n\n# X\n\nCalls `run` somewhere.\n",
    );

    let index = fx.index();
    assert_eq!(index.definitions_of("run").len(), 12);
    assert_eq!(
        linked_symbol_names(&index, &note),
        vec!["anchor"],
        "an identifier with 12 definitions is too ambiguous to link"
    );
}

#[test]
fn fuzzy_links_are_a_labelled_fallback() {
    let fx = Fixture::new("fuzzy");
    fx.write("src/session_manager.rs", SESSION_RS);
    // No file reference and no backtick identifier — only the title to go on.
    let note = fx.note(
        "components/session_manager.md",
        "---\ntype: component\n---\n\n# session_manager\n\nManages sessions.\n",
    );

    let index = fx.index();
    let refs = index.symbols_for_note(&note);
    assert!(
        !refs.is_empty(),
        "fuzzy fallback should still find the file"
    );
    assert!(refs.iter().all(|r| r.confidence == MatchConfidence::Fuzzy));

    let docs = index.docs_for_symbol(&fx.root.join("src/session_manager.rs"), "SessionManager");
    assert!(!docs[0].confidence.is_verified());
    assert!(
        index
            .verified_docs_for_symbol(&fx.root.join("src/session_manager.rs"), "SessionManager")
            .is_empty(),
        "a fuzzy guess must not count as documentation"
    );
}

#[test]
fn an_exact_reference_suppresses_the_fuzzy_fallback() {
    let fx = Fixture::new("no-fuzzy");
    fx.write("src/session.rs", SESSION_RS);
    fx.write("src/session_helper.rs", "pub fn helper() {}\n");
    let note = fx.note(
        "features/session.md",
        "---\ntype: feature\nfiles:\n  - src/session.rs\n---\n\n# session\n",
    );

    let index = fx.index();
    assert!(
        index
            .symbols_for_note(&note)
            .iter()
            .all(|r| r.confidence == MatchConfidence::Exact),
        "a note with a resolved file reference should not also guess by title"
    );
}

#[test]
fn multi_language_symbol_extraction() {
    let fx = Fixture::new("langs");
    fx.write(
        "src/mod.py",
        "class Widget:\n    pass\n\ndef build():\n    pass\n",
    );
    fx.write(
        "src/mod.ts",
        "export interface Shape {}\nexport class Circle {}\n",
    );
    fx.write("src/mod.go", "package main\n\nfunc Run() {}\n");
    fx.note(
        "features/x.md",
        "---\ntype: feature\nfiles:\n  - src/mod.py\n  - src/mod.ts\n  - src/mod.go\n---\n\n# X\n",
    );

    let index = fx.index();
    for name in ["Widget", "build", "Shape", "Circle", "Run"] {
        assert!(
            !index.definitions_of(name).is_empty(),
            "expected to extract `{name}`"
        );
    }
}
