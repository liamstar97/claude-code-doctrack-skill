use std::path::Path;

use tower_lsp::lsp_types::*;

use dt_index::Index;

/// Check a vault note for stale references and return diagnostics.
pub fn check_note(index: &Index, note_path: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let Some(note) = index.vault_notes.get(note_path) else {
        return diagnostics;
    };

    // Check file references resolve to real files
    for file_ref in &note.file_refs {
        let resolved = index.resolve_file_ref(file_ref);
        if resolved.is_empty() {
            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position::new(0, 0),
                    end: Position::new(0, 0),
                },
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("doctrack".to_string()),
                message: format!(
                    "Stale reference: `{}` — not found in project",
                    file_ref.path.display()
                ),
                ..Default::default()
            });
        } else if let Some(line) = file_ref.line {
            for abs_path in &resolved {
                if let Ok(content) = std::fs::read_to_string(abs_path) {
                    let line_count = content.lines().count() as u32;
                    if line > line_count {
                        diagnostics.push(Diagnostic {
                            range: Range {
                                start: Position::new(0, 0),
                                end: Position::new(0, 0),
                            },
                            severity: Some(DiagnosticSeverity::WARNING),
                            source: Some("doctrack".to_string()),
                            message: format!(
                                "Stale line reference: `{}:{}` — file only has {} lines",
                                abs_path.display(),
                                line,
                                line_count
                            ),
                            ..Default::default()
                        });
                    }
                }
            }
        }
    }

    // Check that wikilinked notes still exist in the vault
    for link in &note.wikilinks {
        let linked_path = index.vault_root.join(format!("{link}.md"));
        if !linked_path.exists() {
            // Try case-insensitive search
            let found = index
                .vault_notes
                .iter()
                .any(|entry| entry.value().title.eq_ignore_ascii_case(link));

            if !found {
                diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position::new(0, 0),
                        end: Position::new(0, 0),
                    },
                    severity: Some(DiagnosticSeverity::HINT),
                    source: Some("doctrack".to_string()),
                    message: format!("Broken wikilink: [[{link}]] — note not found in vault"),
                    ..Default::default()
                });
            }
        }
    }

    diagnostics
}
