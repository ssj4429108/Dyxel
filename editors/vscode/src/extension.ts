import * as path from 'path';
import { workspace, ExtensionContext, window, OutputChannel } from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient;
let outputChannel: OutputChannel;

export function activate(context: ExtensionContext) {
  outputChannel = window.createOutputChannel('Dyxel Language Server');
  outputChannel.appendLine('Dyxel extension activating...');

  const config = workspace.getConfiguration('dyxel');
  
  if (!config.get('enableLanguageServer', true)) {
    outputChannel.appendLine('Language server is disabled in settings');
    return;
  }

  const serverPath = config.get<string>('languageServerPath', 'dyxel-lsp');
  outputChannel.appendLine(`Server path: ${serverPath}`);
  
  const serverOptions: ServerOptions = {
    command: serverPath,
    args: [],
    transport: TransportKind.stdio,
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: 'file', language: 'rust' },
      { scheme: 'file', pattern: '**/sample/src/*.rs' },
    ],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher('**/*.rs'),
    },
    outputChannel: outputChannel,
    traceOutputChannel: outputChannel,
  };

  client = new LanguageClient(
    'dyxel',
    'Dyxel Language Server',
    serverOptions,
    clientOptions
  );

  outputChannel.appendLine('Starting language client...');
  client.start().catch(err => {
    outputChannel.appendLine(`Failed to start client: ${err}`);
    window.showErrorMessage(`Dyxel LSP failed to start: ${err}`);
  });
  outputChannel.appendLine('Language client started');
  outputChannel.show();
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
