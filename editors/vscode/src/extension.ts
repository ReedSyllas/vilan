import * as fs from 'fs';
import * as path from 'path';
import { workspace, ExtensionContext } from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

/// Resolve the language-server binary. An explicit `vilan.server.path` setting
/// wins; otherwise look for a binary built in-repo (the extension lives at
/// `<repo>/editors/vscode`, so the cargo target dir is two levels up), and fall
/// back to `vilan-lsp` on PATH.
function resolveServerPath(context: ExtensionContext, configured: string): string {
    if (configured && configured !== 'vilan-lsp') {
        return configured;
    }
    const repoRoot = path.resolve(context.extensionPath, '..', '..');
    for (const profile of ['release', 'debug']) {
        const candidate = path.join(repoRoot, 'target', profile, 'vilan-lsp');
        if (fs.existsSync(candidate)) {
            return candidate;
        }
    }
    return configured || 'vilan-lsp';
}

export function activate(context: ExtensionContext): void {
    const config = workspace.getConfiguration('vilan');
    const command = resolveServerPath(context, config.get<string>('server.path') || 'vilan-lsp');
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
