use std::path::PathBuf;

use anyhow::Result;
use tower_lsp::{LspService, Server};
use tracing::info;
use tracing_subscriber::EnvFilter;

mod capabilities;
mod definition;
mod diagnostics;
mod hover;
mod server;

use server::DoctrackServer;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() >= 3 && args[1] == "--check" {
        return run_check(&args[2]);
    }

    if args.len() >= 2 && matches!(args[1].as_str(), "--version" | "-V") {
        println!(
            "doctrack-lsp {} ({})",
            env!("CARGO_PKG_VERSION"),
            env!("GIT_HASH")
        );
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    info!("starting doctrack-lsp");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(DoctrackServer::new);

    Server::new(stdin, stdout, socket).serve(service).await;

    Ok(())
}

fn run_check(project_path: &str) -> Result<()> {
    use dt_index::Index;

    let root = PathBuf::from(project_path).canonicalize()?;
    let vault_root = root.join(".doctrack");

    if !vault_root.exists() {
        eprintln!("No .doctrack/ vault found at {}", vault_root.display());
        std::process::exit(1);
    }

    println!("Doctrack LSP — Index Check");
    println!("==========================");
    println!("Project root: {}", root.display());
    println!("Vault root:   {}", vault_root.display());
    println!();

    let index = Index::new(root, vault_root);
    index.build()?;

    // --- Vault notes ---
    println!("Vault Notes ({} total)", index.vault_notes.len());
    println!("{}", "-".repeat(60));

    let mut notes: Vec<_> = index
        .vault_notes
        .iter()
        .map(|e| e.value().clone())
        .collect();
    notes.sort_by(|a, b| a.title.cmp(&b.title));

    for note in &notes {
        let file_ref_count = note.file_refs.len();
        let wikilink_count = note.wikilinks.len();
        println!(
            "  {} [{}] — {} file refs, {} wikilinks",
            note.title, note.note_type, file_ref_count, wikilink_count
        );
    }
    println!();

    // --- Code symbols ---
    println!("Code Files ({} indexed)", index.code_symbols.len());
    println!("{}", "-".repeat(60));

    let mut code_files: Vec<_> = index.code_symbols.iter().collect();
    code_files.sort_by(|a, b| a.key().cmp(b.key()));

    for entry in &code_files {
        let file = entry.key();
        let symbols = entry.value();
        println!("  {} ({} symbols)", file.display(), symbols.len());
        for sym in symbols {
            println!(
                "    {} {} (L{}-L{})",
                sym.kind,
                sym.name,
                sym.start_line + 1,
                sym.end_line + 1
            );
        }
    }
    println!();

    // --- Bidirectional links ---
    let total_sym_to_doc: usize = index.sym_to_docs.iter().map(|e| e.value().len()).sum();
    let total_doc_to_sym: usize = index.doc_to_syms.iter().map(|e| e.value().len()).sum();

    println!("Bidirectional Links");
    println!("{}", "-".repeat(60));
    println!(
        "  Symbol → Doc: {} symbols linked to {} doc references",
        index.sym_to_docs.len(),
        total_sym_to_doc
    );
    println!(
        "  Doc → Symbol: {} notes linked to {} symbol references",
        index.doc_to_syms.len(),
        total_doc_to_sym
    );
    println!();

    // --- Symbol → Doc detail ---
    if !index.sym_to_docs.is_empty() {
        println!("Symbol → Doc Mappings");
        println!("{}", "-".repeat(60));

        let mut mappings: Vec<_> = index.sym_to_docs.iter().collect();
        mappings.sort_by(|a, b| a.key().file.cmp(&b.key().file));

        for entry in &mappings {
            let id = entry.key();
            let docs = entry.value();
            println!("  {}::{}", id.file.display(), id.name);
            for doc in docs {
                println!(
                    "    → {} [{}] — {}",
                    doc.note_title, doc.note_type, doc.context
                );
            }
        }
        println!();
    }

    // --- Stale references ---
    let mut stale_count = 0;
    let mut resolved_count = 0;
    println!("Stale Reference Check");
    println!("{}", "-".repeat(60));

    for note in &notes {
        for file_ref in &note.file_refs {
            let resolved = index.resolve_file_ref(file_ref);
            if resolved.is_empty() {
                println!(
                    "  STALE: {} references `{}`{} — not found",
                    note.title,
                    file_ref.path.display(),
                    if file_ref.is_bare_filename {
                        " (bare filename)"
                    } else {
                        ""
                    }
                );
                stale_count += 1;
            } else {
                resolved_count += 1;
                // Check line validity on resolved files
                if let Some(line) = file_ref.line {
                    for abs in &resolved {
                        if let Ok(content) = std::fs::read_to_string(abs) {
                            let line_count = content.lines().count() as u32;
                            if line > line_count {
                                println!(
                                    "  STALE LINE: {} references `{}:{}` — file only has {} lines",
                                    note.title,
                                    abs.display(),
                                    line,
                                    line_count
                                );
                                stale_count += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    if stale_count == 0 {
        println!("  No stale references found.");
    }
    println!("  {} resolved, {} stale", resolved_count, stale_count);
    println!();

    // --- Summary ---
    println!("Summary");
    println!("{}", "-".repeat(60));
    println!("  Vault notes:      {}", notes.len());
    println!("  Code files:       {}", index.code_symbols.len());
    println!("  Sym→Doc links:    {}", total_sym_to_doc);
    println!("  Doc→Sym links:    {}", total_doc_to_sym);
    println!("  Stale refs:       {}", stale_count);

    let coverage = if !notes.is_empty() {
        (index.doc_to_syms.len() as f64 / notes.len() as f64 * 100.0) as u32
    } else {
        0
    };
    println!("  Link coverage:    {}% of notes have code links", coverage);

    Ok(())
}
