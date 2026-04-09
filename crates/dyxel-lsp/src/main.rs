// Dyxel Language Server - Simplified Version
// 提供 RSX 基础支持

use dyxel_lsp::RsxAnalyzer;
use std::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug)]
struct DyxelLanguageServer {
    client: Client,
    analyzer: Mutex<RsxAnalyzer>,
}

#[tower_lsp::async_trait]
impl LanguageServer for DyxelLanguageServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        ..Default::default()
                    },
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string(), ":".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Dyxel LSP initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;

        self.client
            .log_message(MessageType::INFO, format!("Opened: {}", uri))
            .await;

        // 打开文档到分析器
        if let Ok(mut analyzer) = self.analyzer.lock() {
            analyzer.open_document(&uri, &text);
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;

        self.client
            .log_message(MessageType::INFO, format!("Changed: {}", uri))
            .await;

        // 更新文档内容
        if let Some(change) = params.content_changes.first() {
            if let Ok(mut analyzer) = self.analyzer.lock() {
                analyzer.update_document(&uri, &change.text);
            }
        }
    }

    // 处理补全请求
    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        self.client
            .log_message(
                MessageType::INFO,
                format!("Completion requested at {:?}", position),
            )
            .await;

        let items = {
            if let Ok(analyzer) = self.analyzer.lock() {
                analyzer.complete(&uri, position)
            } else {
                vec![]
            }
        };

        self.client
            .log_message(
                MessageType::INFO,
                format!("Returning {} completion items", items.len()),
            )
            .await;

        Ok(Some(CompletionResponse::Array(items)))
    }

    // 处理定义跳转
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let locations = {
            if let Ok(analyzer) = self.analyzer.lock() {
                analyzer.find_definition(&uri, position)
            } else {
                vec![]
            }
        };

        if !locations.is_empty() {
            Ok(Some(GotoDefinitionResponse::Array(locations)))
        } else {
            Ok(None)
        }
    }

    // 处理悬停
    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let result = {
            if let Ok(analyzer) = self.analyzer.lock() {
                analyzer.hover(&uri, position)
            } else {
                None
            }
        };

        Ok(result)
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| DyxelLanguageServer {
        client,
        analyzer: Mutex::new(RsxAnalyzer::new()),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
