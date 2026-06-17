import { workspace, ExtensionContext } from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

export function activate(_context: ExtensionContext): void {
    const config = workspace.getConfiguration('vilan');
    const command = config.get<string>('server.path') || 'vilan-lsp';
    const stdPath = config.get<string>('stdPath') || '';

    const env = { ...process.env };
    if (stdPath) {
        env.VILAN_STD = stdPath;
    }

    const run = { command, transport: TransportKind.stdio, options: { env } };
    const serverOptions: ServerOptions = { run, debug: run };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'vilan' }],
        synchronize: {
            fileEvents: workspace.createFileSystemWatcher('**/*.vl'),
        },
    };

    client = new LanguageClient(
        'vilan',
        'Vilan Language Server',
        serverOptions,
        clientOptions,
    );
    client.start();
}

export function deactivate(): Thenable<void> | undefined {
    return client?.stop();
}
