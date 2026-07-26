use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use tracing::info;

use dt_index::Index;
use dt_watch::{FileWatcher, WatchEvent};

use crate::capabilities;
use crate::definition;
use crate::diagnostics;
use crate::hover;

pub struct DoctrackServer {
    client: Client,
    index: Arc<ArcSwap<Index>>,
}

impl DoctrackServer {
    pub fn new(client: Client) -> Self {
        let index = Arc::new(ArcSwap::from_pointee(Index::new(
            PathBuf::new(),
            PathBuf::new(),
        )));
        Self { client, index }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for DoctrackServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        info!("doctrack-lsp initializing");

        if let Some(folders) = &params.workspace_folders
            && let Some(folder) = folders.first()
        {
            // `uri.path()` is percent-encoded: a workspace at
            // "/home/me/My Project" would come back as ".../My%20Project"
            // and every path built from it would miss.
            let Ok(root) = folder.uri.to_file_path() else {
                info!("workspace folder is not a local path: {}", folder.uri);
                return Ok(InitializeResult {
                    capabilities: capabilities::server_capabilities(),
                    server_info: Some(ServerInfo {
                        name: "doctrack-lsp".to_string(),
                        version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    }),
                });
            };
            let vault_root = root.join(".doctrack");

            if vault_root.exists() {
                info!("found vault at {}", vault_root.display());

                let new_index = Index::new(root.clone(), vault_root.clone());
                if let Err(e) = new_index.build() {
                    info!("index build error: {e}");
                }

                // Atomically swap in the built index
                self.index.store(Arc::new(new_index));

                // Start file watcher with shared index and client
                let index = self.index.clone();
                let client = self.client.clone();
                tokio::spawn(async move {
                    run_watcher(index, client, vault_root, root).await;
                });
            } else {
                info!("no .doctrack/ vault found in workspace");
            }
        }

        Ok(InitializeResult {
            capabilities: capabilities::server_capabilities(),
            server_info: Some(ServerInfo {
                name: "doctrack-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("doctrack-lsp initialized");
    }

    async fn shutdown(&self) -> Result<()> {
        info!("doctrack-lsp shutting down");
        Ok(())
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let index = self.index.load();
        Ok(hover::handle_hover(&index, params))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let index = self.index.load();
        Ok(definition::handle_goto_definition(&index, params))
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let index = self.index.load();
        let uri = params.text_document.uri;
        let Ok(path) = uri.to_file_path() else { return };

        if !index.code_symbols.contains_key(&path) {
            let _ = index.reindex_code_file(&path);
        }

        if path.starts_with(&index.vault_root) {
            let diags = diagnostics::check_note(&index, &path);
            self.client.publish_diagnostics(uri, diags, None).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let index = self.index.load();
        let uri = params.text_document.uri;
        let Ok(path) = uri.to_file_path() else { return };

        if path.starts_with(&index.vault_root) {
            let _ = index.reindex_note(&path);
            let diags = diagnostics::check_note(&index, &path);
            self.client.publish_diagnostics(uri, diags, None).await;
        } else {
            let _ = index.reindex_code_file(&path);
        }
    }
}

async fn run_watcher(
    index: Arc<ArcSwap<Index>>,
    client: Client,
    vault_root: PathBuf,
    project_root: PathBuf,
) {
    let idx = index.load();
    let watcher = FileWatcher::new(
        // The watcher needs an Arc<Index> for reindexing — give it the current one.
        // DashMap mutations are visible through the Arc even after ArcSwap loads.
        Arc::clone(&arc_swap::Guard::into_inner(idx)),
        vault_root,
        project_root,
    );

    match watcher.start().await {
        Ok(mut rx) => {
            info!("file watcher started");
            while let Some(event) = rx.recv().await {
                let idx = index.load();
                match &event {
                    WatchEvent::VaultNoteChanged(path) => {
                        info!("vault note changed: {}", path.display());
                        if let Ok(uri) = Url::from_file_path(path) {
                            let diags = diagnostics::check_note(&idx, path);
                            client.publish_diagnostics(uri, diags, None).await;
                        }
                    }
                    WatchEvent::VaultNoteRemoved(path) => {
                        info!("vault note removed: {}", path.display());
                        idx.remove_note(path);
                        if let Ok(uri) = Url::from_file_path(path) {
                            client.publish_diagnostics(uri, vec![], None).await;
                        }
                    }
                    WatchEvent::CodeFileChanged(path) => {
                        info!("code file changed: {}", path.display());
                    }
                    WatchEvent::CodeFileRemoved(path) => {
                        info!("code file removed: {}", path.display());
                        idx.remove_code_file(path);
                    }
                }
            }
        }
        Err(e) => {
            info!("file watcher failed to start: {e}");
        }
    }
}
