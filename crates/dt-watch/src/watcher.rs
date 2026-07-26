use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use notify_debouncer_mini::{DebouncedEventKind, new_debouncer};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use dt_index::Index;

/// Events emitted by the file watcher.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// A vault note was created or modified.
    VaultNoteChanged(PathBuf),
    /// A vault note was removed.
    VaultNoteRemoved(PathBuf),
    /// A code file was created or modified.
    CodeFileChanged(PathBuf),
    /// A code file was removed.
    CodeFileRemoved(PathBuf),
}

/// Watches both the vault and project directories for changes.
pub struct FileWatcher {
    index: Arc<Index>,
    vault_root: PathBuf,
    project_root: PathBuf,
}

impl FileWatcher {
    pub fn new(index: Arc<Index>, vault_root: PathBuf, project_root: PathBuf) -> Self {
        Self {
            index,
            vault_root,
            project_root,
        }
    }

    /// Start watching for file changes. Returns a channel of watch events.
    pub async fn start(&self) -> Result<mpsc::UnboundedReceiver<WatchEvent>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let vault_root = self.vault_root.clone();
        let project_root = self.project_root.clone();
        let index = self.index.clone();

        // Spawn a blocking thread for the file watcher
        tokio::task::spawn_blocking(move || {
            let (notify_tx, notify_rx) = std::sync::mpsc::channel();

            // A watcher that can't start is a degraded index, not a reason to
            // take down the MCP or LSP server that spawned this thread.
            let mut debouncer = match new_debouncer(Duration::from_millis(500), notify_tx) {
                Ok(d) => d,
                Err(e) => {
                    warn!("failed to create file watcher: {e}");
                    return;
                }
            };

            // Watch the vault directory
            if let Err(e) = debouncer
                .watcher()
                .watch(&vault_root, notify::RecursiveMode::Recursive)
            {
                warn!("failed to watch vault {}: {e}", vault_root.display());
                return;
            }

            // Watch the project root recursively — events are filtered by path below
            if let Err(e) = debouncer
                .watcher()
                .watch(&project_root, notify::RecursiveMode::Recursive)
            {
                warn!("failed to watch project {}: {e}", project_root.display());
                return;
            }

            info!(
                "watching vault={} project={}",
                vault_root.display(),
                project_root.display()
            );

            for result in notify_rx {
                match result {
                    Ok(events) => {
                        for event in events {
                            if event.kind != DebouncedEventKind::Any {
                                continue;
                            }

                            let path = &event.path;

                            // Obsidian churns through .obsidian/ and .trash/ as
                            // you browse; those aren't notes.
                            if path.starts_with(&vault_root) {
                                if is_markdown(path) && !dt_index::vault::is_vault_internal(path) {
                                    if path.exists() {
                                        debug!("vault note changed: {}", path.display());
                                        let _ = index.reindex_note(path);
                                        let _ = tx.send(WatchEvent::VaultNoteChanged(path.clone()));
                                    } else {
                                        debug!("vault note removed: {}", path.display());
                                        index.remove_note(path);
                                        let _ = tx.send(WatchEvent::VaultNoteRemoved(path.clone()));
                                    }
                                }
                            } else if dt_index::symbols::is_supported(path) {
                                if path.exists() {
                                    debug!("code file changed: {}", path.display());
                                    let _ = index.reindex_code_file(path);
                                    let _ = tx.send(WatchEvent::CodeFileChanged(path.clone()));
                                } else {
                                    debug!("code file removed: {}", path.display());
                                    index.remove_code_file(path);
                                    let _ = tx.send(WatchEvent::CodeFileRemoved(path.clone()));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("file watcher error: {:?}", e);
                    }
                }
            }
        });

        Ok(rx)
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "md")
}
