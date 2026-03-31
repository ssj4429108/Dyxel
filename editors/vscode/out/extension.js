"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.deactivate = exports.activate = void 0;
const vscode_1 = require("vscode");
const node_1 = require("vscode-languageclient/node");
let client;
let outputChannel;
function activate(context) {
    outputChannel = vscode_1.window.createOutputChannel('Dyxel Language Server');
    outputChannel.appendLine('Dyxel extension activating...');
    const config = vscode_1.workspace.getConfiguration('dyxel');
    if (!config.get('enableLanguageServer', true)) {
        outputChannel.appendLine('Language server is disabled in settings');
        return;
    }
    const serverPath = config.get('languageServerPath', 'dyxel-lsp');
    outputChannel.appendLine(`Server path: ${serverPath}`);
    const serverOptions = {
        command: serverPath,
        args: [],
        transport: node_1.TransportKind.stdio,
    };
    const clientOptions = {
        documentSelector: [
            { scheme: 'file', language: 'rust' },
            { scheme: 'file', pattern: '**/sample/src/*.rs' },
        ],
        synchronize: {
            fileEvents: vscode_1.workspace.createFileSystemWatcher('**/*.rs'),
        },
        outputChannel: outputChannel,
        traceOutputChannel: outputChannel,
    };
    client = new node_1.LanguageClient('dyxel', 'Dyxel Language Server', serverOptions, clientOptions);
    outputChannel.appendLine('Starting language client...');
    client.start().catch(err => {
        outputChannel.appendLine(`Failed to start client: ${err}`);
        vscode_1.window.showErrorMessage(`Dyxel LSP failed to start: ${err}`);
    });
    outputChannel.appendLine('Language client started');
    outputChannel.show();
}
exports.activate = activate;
function deactivate() {
    if (!client) {
        return undefined;
    }
    return client.stop();
}
exports.deactivate = deactivate;
//# sourceMappingURL=extension.js.map