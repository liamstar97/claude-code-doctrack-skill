use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rmcp::ServiceExt;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod tools;

use tools::DoctrackMcp;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // CLI modes — build index, run check, print results, exit
    if args.len() >= 3 {
        match args[1].as_str() {
            "--check-impact" => return run_check_impact(&args[2]),
            "--validate-note" => return run_validate_note(&args[2]),
            _ => {}
        }
    }
    if args.len() >= 2 {
        match args[1].as_str() {
            "--coverage" => return run_coverage(),
            "--setup-hooks" => return setup_hooks(),
            "--version" | "-V" => {
                print_version();
                return Ok(());
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            "--update" => return run_update(),
            _ => {}
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    info!("starting doctrack-mcp server");

    let root = project_root();
    let vault_root = root.join(".doctrack");

    let server = DoctrackMcp::new(root, vault_root)?;

    let transport = rmcp::transport::stdio();

    let service = server.serve(transport).await?;
    service.waiting().await?;

    Ok(())
}

fn project_root() -> PathBuf {
    std::env::var("DOCTRACK_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default())
}

/// CLI: --check-impact <file>
/// After modifying a code file, report which vault notes may need updating.
fn run_check_impact(file: &str) -> Result<()> {
    let root = project_root();
    let vault_root = root.join(".doctrack");

    if !vault_root.exists() {
        return Ok(());
    }

    let index = Arc::new(dt_index::Index::new(root.clone(), vault_root));
    index.build()?;

    let abs_path = root.join(file);
    let _ = index.reindex_code_file(&abs_path);

    let mut impacted = Vec::new();

    // Check symbol→doc links
    if let Some(symbols) = index.code_symbols.get(&abs_path) {
        for sym in symbols.value() {
            for doc in index.docs_for_symbol(&abs_path, &sym.name) {
                impacted.push(format!(
                    "  - {} [{}] references `{}`",
                    doc.note_title, doc.note_type, sym.name
                ));
            }
        }
    }

    // Check notes that reference the file path directly
    let filename = abs_path.file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    for entry in index.vault_notes.iter() {
        let note = entry.value();
        for file_ref in &note.file_refs {
            let ref_str = file_ref.path.to_string_lossy();
            if ref_str.ends_with(&filename) || ref_str == file {
                let line = format!(
                    "  - {} [{}] has file reference to `{}`",
                    note.title, note.note_type, file_ref.path.display()
                );
                if !impacted.contains(&line) {
                    impacted.push(line);
                }
            }
        }
    }

    impacted.dedup();

    if !impacted.is_empty() {
        println!(
            "Doctrack: changes to `{}` may affect {} vault note(s):\n{}",
            file,
            impacted.len(),
            impacted.join("\n")
        );
    }

    Ok(())
}

/// CLI: --validate-note <note>
/// Check a vault note for stale references and broken wikilinks.
fn run_validate_note(note: &str) -> Result<()> {
    let root = project_root();
    let vault_root = root.join(".doctrack");

    if !vault_root.exists() {
        return Ok(());
    }

    let index = Arc::new(dt_index::Index::new(root, vault_root.clone()));
    index.build()?;

    let note_path = vault_root.join(note);
    let _ = index.reindex_note(&note_path);

    let Some(vault_note) = index.vault_notes.get(&note_path) else {
        eprintln!("Note not found: {note}");
        return Ok(());
    };

    let mut issues = Vec::new();

    for file_ref in &vault_note.file_refs {
        let paths = index.resolve_file_ref(file_ref);
        if paths.is_empty() {
            issues.push(format!(
                "  - STALE: `{}` not found in project",
                file_ref.path.display()
            ));
        } else if paths.len() > 1 {
            issues.push(format!(
                "  - AMBIGUOUS: `{}` resolves to {} files",
                file_ref.path.display(),
                paths.len()
            ));
        }
    }

    for link in &vault_note.wikilinks {
        let linked_path = vault_root.join(format!("{link}.md"));
        if !linked_path.exists() {
            let found = index.vault_notes.iter().any(|e| {
                e.value().title.eq_ignore_ascii_case(link)
            });
            if !found {
                issues.push(format!("  - BROKEN WIKILINK: [[{link}]]"));
            }
        }
    }

    if !issues.is_empty() {
        println!(
            "Doctrack: {} has {} issue(s):\n{}",
            note,
            issues.len(),
            issues.join("\n")
        );
    }

    Ok(())
}

/// CLI: --coverage
/// Quick coverage summary.
fn run_coverage() -> Result<()> {
    let root = project_root();
    let vault_root = root.join(".doctrack");

    if !vault_root.exists() {
        return Ok(());
    }

    let index = Arc::new(dt_index::Index::new(root, vault_root));
    index.build()?;

    let total_notes = index.vault_notes.len();
    let total_code = index.code_symbols.len();
    let linked = index.doc_to_syms.len();
    let total_links: usize = index.sym_to_docs.iter().map(|e| e.value().len()).sum();
    let coverage = if total_notes > 0 {
        (linked as f64 / total_notes as f64 * 100.0) as u32
    } else {
        0
    };

    println!(
        "Doctrack: {} notes, {} code files, {} links, {}% coverage",
        total_notes, total_code, total_links, coverage
    );

    Ok(())
}

fn print_version() {
    println!(
        "doctrack-mcp {} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("GIT_HASH")
    );
}

fn print_help() {
    println!(
        "doctrack-mcp {} ({})

{}

USAGE:
    doctrack-mcp [OPTION]

With no arguments, starts the MCP server on stdio — this is how Claude Code
launches it. Each OPTION below runs a single one-shot command and exits.

OPTIONS:
    --check-impact <FILE>     Report vault notes affected by changes to FILE
    --validate-note <NOTE>    Check a vault note for stale refs / broken wikilinks
    --coverage                Print a one-line vault coverage summary
    --setup-hooks             Install Claude Code hooks into .claude/settings.json
    --update                  Reinstall doctrack binaries from GitHub main
    -V, --version             Print version and git hash
    -h, --help                Print this help

ENVIRONMENT:
    DOCTRACK_ROOT             Project root (defaults to the current directory)",
        env!("CARGO_PKG_VERSION"),
        env!("GIT_HASH"),
        env!("CARGO_PKG_DESCRIPTION"),
    );
}

/// CLI: --update
/// Reinstall both doctrack binaries from GitHub main.
fn run_update() -> Result<()> {
    print_version();
    println!("Updating from GitHub...\n");

    let repo = "https://github.com/liamstar97/doctrack.git";

    println!("Installing dt-mcp...");
    let mcp_status = std::process::Command::new("cargo")
        .args(["install", "--git", repo, "dt-mcp", "--force"])
        .status();

    println!("\nInstalling dt-lsp...");
    let lsp_status = std::process::Command::new("cargo")
        .args(["install", "--git", repo, "dt-lsp", "--force"])
        .status();

    println!();
    match (mcp_status, lsp_status) {
        (Ok(m), Ok(l)) if m.success() && l.success() => {
            println!("Both binaries updated successfully.");
            println!("Restart Claude Code and your editor for changes to take effect.");
        }
        _ => {
            println!("Some installations may have failed — check output above.");
        }
    }

    Ok(())
}

/// CLI: --setup-hooks
/// Install Claude Code hooks for proactive code↔doc feedback.
fn setup_hooks() -> Result<()> {
    let root = project_root();
    let settings_dir = root.join(".claude");
    let settings_path = settings_dir.join("settings.json");

    // Read existing settings or start fresh
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));

    // Use the binary name on PATH, not an absolute path — absolute paths
    // break across users and machines when hooks are committed to git.
    let bin_path = "doctrack-mcp";

    // SessionStart hook — coverage summary
    let session_start_hook = serde_json::json!({
        "hooks": [{
            "type": "command",
            "command": format!(
                "if [ -d .doctrack ]; then DOCTRACK_ROOT=\"$(pwd)\" {bin_path} --coverage 2>/dev/null; fi"
            ),
            "statusMessage": "Doctrack: checking vault..."
        }]
    });

    // PostToolUse hook — validate notes after obsidian writes
    // Hook receives JSON on stdin with tool_input.path
    let post_tool_hook = serde_json::json!({
        "matcher": "mcp__obsidian__write_note|mcp__obsidian__patch_note",
        "hooks": [{
            "type": "command",
            "command": format!(
                "NOTE=$(cat | jq -r '.tool_input.path // empty'); if [ -n \"$NOTE\" ] && [ -d .doctrack ]; then DOCTRACK_ROOT=\"$(pwd)\" {bin_path} --validate-note \"$NOTE\" 2>/dev/null; fi"
            ),
            "statusMessage": "Doctrack: validating note..."
        }]
    });

    // Merge SessionStart — append if no doctrack hook exists yet
    let session_start = hooks
        .as_object_mut()
        .unwrap()
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]));

    let has_doctrack_session = session_start
        .as_array()
        .map(|arr| arr.iter().any(|h| {
            h.to_string().contains("doctrack")
        }))
        .unwrap_or(false);

    if !has_doctrack_session {
        session_start.as_array_mut().unwrap().push(session_start_hook);
    }

    // Merge PostToolUse — append if no doctrack hook exists yet
    let post_tool = hooks
        .as_object_mut()
        .unwrap()
        .entry("PostToolUse")
        .or_insert_with(|| serde_json::json!([]));

    let has_doctrack_post = post_tool
        .as_array()
        .map(|arr| arr.iter().any(|h| {
            h.to_string().contains("doctrack")
        }))
        .unwrap_or(false);

    if !has_doctrack_post {
        post_tool.as_array_mut().unwrap().push(post_tool_hook);
    }

    // Write back
    std::fs::create_dir_all(&settings_dir)?;
    let formatted = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, formatted)?;

    println!("Doctrack hooks installed in {}", settings_path.display());
    println!("  - SessionStart: vault coverage summary");
    println!("  - PostToolUse: validate notes after writing");
    println!("\nRestart Claude Code for hooks to take effect.");

    Ok(())
}
