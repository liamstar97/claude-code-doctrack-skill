use tower_lsp::lsp_types::*;

use dt_index::Index;

/// Handle textDocument/hover — show doctrack note summaries for code symbols.
pub fn handle_hover(index: &Index, params: HoverParams) -> Option<Hover> {
    let uri = &params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;
    let path = uri.to_file_path().ok()?;

    // For code files: find the symbol at the cursor position and show linked docs
    if let Some(symbols) = index.code_symbols.get(&path) {
        let line = position.line;

        for sym in symbols.value() {
            if line >= sym.start_line && line <= sym.end_line {
                let docs = index.docs_for_symbol(&path, &sym.name);
                if !docs.is_empty() {
                    let mut content = format!("### {} `{}`\n\n", sym.kind, sym.name);
                    content.push_str("**Documented in:**\n\n");

                    for doc in &docs {
                        // Fuzzy links are title-similarity guesses. Say so
                        // rather than presenting them as documentation.
                        let hint = if doc.confidence.is_verified() {
                            String::new()
                        } else {
                            format!(" _({} match)_", doc.confidence)
                        };
                        content.push_str(&format!(
                            "- **{}** ({}): {}{}\n",
                            doc.note_title, doc.note_type, doc.context, hint
                        ));
                    }

                    return Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: content,
                        }),
                        range: Some(Range {
                            start: Position::new(sym.start_line, 0),
                            end: Position::new(sym.end_line, 0),
                        }),
                    });
                }
            }
        }
    }

    // For vault notes: show code symbol info for file references
    if path.starts_with(&index.vault_root) {
        let refs = index.symbols_for_note(&path);
        if !refs.is_empty() {
            let mut content = String::from("### Linked code symbols\n\n");
            for sym_ref in &refs {
                content.push_str(&format!(
                    "- `{}` in `{}` ({} match)\n",
                    sym_ref.symbol_id.name,
                    sym_ref.symbol_id.file.display(),
                    sym_ref.confidence
                ));
            }

            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content,
                }),
                range: None,
            });
        }
    }

    None
}
