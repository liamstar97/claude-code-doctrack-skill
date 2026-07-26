use std::collections::HashSet;
use std::path::PathBuf;

use tracing::debug;

use crate::index::{DocLink, Index, MatchConfidence, SymbolId, SymbolRef};
use crate::vault::VaultNote;

/// Minimum nucleo score for a note title to be considered a fuzzy match for a
/// filename. Fuzzy links are a navigation hint of last resort, so the bar is
/// deliberately higher than a bare "the characters appear in order".
const FUZZY_SCORE_THRESHOLD: u32 = 80;

/// Cap on how many definitions a single backtick identifier may link to.
///
/// Now that the whole project is indexed, a note mentioning `` `new` `` or
/// `` `run` `` would otherwise attach itself to every definition of that name in
/// the codebase. Past this fan-out the reference carries no information about
/// which one is meant, so linking none is more honest than linking all.
const MAX_STRONG_FANOUT: usize = 8;

/// Identifiers below this length are too generic to be a useful reference.
const MIN_IDENT_LEN: usize = 3;

/// Build all bidirectional links between vault notes and code symbols.
pub fn link_all(index: &Index) {
    let notes: Vec<VaultNote> = index
        .vault_notes
        .iter()
        .map(|e| e.value().clone())
        .collect();
    for note in &notes {
        link_note(index, note);
    }
}

/// Link a single note to the code symbols it references.
///
/// Only adds links — callers re-linking an existing note must
/// [`Index::unlink_note`] first.
pub fn link_note(index: &Index, note: &VaultNote) {
    let mut refs: Vec<SymbolRef> = Vec::new();
    let mut seen: HashSet<SymbolId> = HashSet::new();

    // A file listed in frontmatter and mentioned again in prose is one link, not
    // two — and the strongest tier wins because tiers run in order.
    macro_rules! record {
        ($id:expr, $confidence:expr, $context:expr) => {{
            let id: SymbolId = $id;
            let confidence: MatchConfidence = $confidence;
            if seen.insert(id.clone()) {
                refs.push(SymbolRef {
                    symbol_id: id.clone(),
                    confidence,
                });
                index.sym_to_docs.entry(id).or_default().push(DocLink {
                    note_path: note.path.clone(),
                    note_title: note.title.clone(),
                    note_type: note.note_type.clone(),
                    context: $context,
                    confidence,
                });
            }
        }};
    }

    // Tier 1: exact path matches from file references (resolved via lookup)
    for file_ref in &note.file_refs {
        for abs_path in index.resolve_file_ref(file_ref) {
            let Some(symbols) = index.code_symbols.get(&abs_path) else {
                continue;
            };
            for sym in symbols.value() {
                // If a specific line is referenced, only link symbols at that line
                let matches_line = file_ref
                    .line
                    .is_none_or(|line| line >= sym.start_line && line <= sym.end_line);
                if !matches_line {
                    continue;
                }
                record!(
                    SymbolId {
                        file: abs_path.clone(),
                        name: sym.name.clone(),
                    },
                    MatchConfidence::Exact,
                    note.summary.clone()
                );
            }
        }
    }

    // Tier 2: symbol name matches from backtick references in the note body
    for name in &note.code_idents {
        if name.len() < MIN_IDENT_LEN {
            continue;
        }
        let definitions = index.definitions_of(name);
        if definitions.len() > MAX_STRONG_FANOUT {
            debug!(
                "note '{}' mentions `{name}`, which has {} definitions — too ambiguous to link",
                note.title,
                definitions.len()
            );
            continue;
        }
        for id in definitions {
            record!(id, MatchConfidence::Strong, format!("references `{name}`"));
        }
    }

    // Tier 3: fuzzy title/filename matching — a fallback only. If the note
    // already resolved a real file reference we trust that instead of guessing,
    // and even then we take only the single best-scoring file rather than every
    // file whose name loosely resembles the title.
    let has_exact = refs.iter().any(|r| r.confidence == MatchConfidence::Exact);
    if !has_exact && let Some((file, score)) = best_fuzzy_file(&note.title, index) {
        {
            let names: Vec<String> = index
                .code_symbols
                .get(&file)
                .map(|s| s.value().iter().map(|sym| sym.name.clone()).collect())
                .unwrap_or_default();

            if !names.is_empty() {
                debug!(
                    "fuzzy match: note '{}' -> {} (score: {})",
                    note.title,
                    file.display(),
                    score
                );
            }
            for name in names {
                record!(
                    SymbolId {
                        file: file.clone(),
                        name,
                    },
                    MatchConfidence::Fuzzy,
                    format!("fuzzy title match (score: {score})")
                );
            }
        }
    }

    if !refs.is_empty() {
        index.doc_to_syms.insert(note.path.clone(), refs);
    }
}

/// Find the single best fuzzy match between a note title and an indexed file's
/// name. Returns `None` when nothing clears [`FUZZY_SCORE_THRESHOLD`].
fn best_fuzzy_file(title: &str, index: &Index) -> Option<(PathBuf, u32)> {
    use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
    use nucleo_matcher::{Config, Matcher};

    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(title, CaseMatching::Ignore, Normalization::Smart);

    let mut best: Option<(PathBuf, u32)> = None;

    for entry in index.code_symbols.iter() {
        let file = entry.key();
        let Some(stem) = file.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let mut buf = Vec::new();
        let Some(score) =
            pattern.score(nucleo_matcher::Utf32Str::new(stem, &mut buf), &mut matcher)
        else {
            continue;
        };
        if score < FUZZY_SCORE_THRESHOLD {
            continue;
        }
        if best.as_ref().is_none_or(|(_, b)| score > *b) {
            best = Some((file.clone(), score));
        }
    }

    best
}
