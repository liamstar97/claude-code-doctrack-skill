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
    let args: Vec<String> = std::env::args().skip(1).collect();

    // CLI modes — build index, run check, print results, exit.
    //
    // Anything that isn't a recognised, well-formed invocation must fail loudly.
    // Falling through to the server means a typo'd flag hangs on stdio waiting
    // for an MCP handshake that will never come.
    if let Some(flag) = args.first() {
        let arg = args.get(1);
        match flag.as_str() {
            "--check-impact" => return require_arg(flag, arg).and_then(run_check_impact),
            "--validate-note" => return require_arg(flag, arg).and_then(run_validate_note),
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
            other => {
                eprintln!("doctrack-mcp: unrecognised option `{other}`\n");
                print_help();
                std::process::exit(2);
            }
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

/// Exit with usage rather than silently starting a server when a flag that
/// needs a value was given none.
fn require_arg<'a>(flag: &str, value: Option<&'a String>) -> Result<&'a str> {
    match value {
        Some(v) if !v.starts_with('-') => Ok(v.as_str()),
        _ => {
            eprintln!("doctrack-mcp: `{flag}` requires an argument\n");
            print_help();
            std::process::exit(2);
        }
    }
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
            for doc in index.verified_docs_for_symbol(&abs_path, &sym.name) {
                let line = format!(
                    "  - {} [{}] references `{}`",
                    doc.note_title, doc.note_type, sym.name
                );
                if !impacted.contains(&line) {
                    impacted.push(line);
                }
            }
        }
    }

    // Check notes that reference the file path directly
    for entry in index.vault_notes.iter() {
        let note = entry.value();
        for file_ref in &note.file_refs {
            if !dt_index::vault::ref_matches_path(&file_ref.path, &abs_path) {
                continue;
            }
            let line = format!(
                "  - {} [{}] has file reference to `{}`",
                note.title,
                note.note_type,
                file_ref.path.display()
            );
            if !impacted.contains(&line) {
                impacted.push(line);
            }
        }
    }

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
            let found = index
                .vault_notes
                .iter()
                .any(|e| e.value().title.eq_ignore_ascii_case(link));
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
    // Fuzzy title guesses are excluded — they'd otherwise inflate the one
    // number a user sees at every session start.
    let linked = index
        .doc_to_syms
        .iter()
        .filter(|e| e.value().iter().any(|r| r.confidence.is_verified()))
        .count();
    let total_links: usize = index
        .sym_to_docs
        .iter()
        .map(|e| {
            e.value()
                .iter()
                .filter(|d| d.confidence.is_verified())
                .count()
        })
        .sum();
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

    // Read existing settings or start fresh. A settings file we can't parse is
    // the user's, not ours — refuse rather than silently replacing it.
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        if content.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&content).map_err(|e| {
                anyhow::anyhow!(
                    "{} is not valid JSON ({e}) — fix or remove it, then re-run --setup-hooks",
                    settings_path.display()
                )
            })?
        }
    } else {
        serde_json::json!({})
    };

    let Some(root_obj) = settings.as_object_mut() else {
        anyhow::bail!(
            "{} must contain a JSON object at the top level",
            settings_path.display()
        );
    };

    let hooks = root_obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        anyhow::bail!(
            "{} has a non-object `hooks` entry — refusing to overwrite it",
            settings_path.display()
        );
    }

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

    // Merge each event — append only if no doctrack hook exists there yet
    for (event, hook) in [
        ("SessionStart", session_start_hook),
        ("PostToolUse", post_tool_hook),
    ] {
        let hooks_obj = hooks
            .as_object_mut()
            .expect("checked to be an object above");
        let entry = hooks_obj
            .entry(event)
            .or_insert_with(|| serde_json::json!([]));

        let Some(arr) = entry.as_array_mut() else {
            anyhow::bail!(
                "{} has a non-array `hooks.{event}` entry — refusing to overwrite it",
                settings_path.display()
            );
        };

        if !arr.iter().any(|h| h.to_string().contains("doctrack")) {
            arr.push(hook);
        }
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
