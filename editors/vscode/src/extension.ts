import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { workspace, window, commands, ExtensionContext } from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

/// Resolve the language-server binary. An explicit `vilan.server.path` setting
/// wins; otherwise look for a binary built in-repo (the extension lives at
/// `<repo>/editors/vscode`, so the cargo target dir is two levels up) or
/// installed by `cargo install` (`~/.cargo/bin`), and fall back to `vilan-lsp`
/// on PATH.
function resolveServerPath(context: ExtensionContext, configured: string): string {
    if (configured && configured !== 'vilan-lsp') {
        return configured;
    }
    const repoRoot = path.resolve(context.extensionPath, '..', '..');
    const candidates = [
        path.join(repoRoot, 'target', 'release', 'vilan-lsp'),
        path.join(repoRoot, 'target', 'debug', 'vilan-lsp'),
        path.join(os.homedir(), '.cargo', 'bin', 'vilan-lsp'),
    ];
    for (const candidate of candidates) {
        if (fs.existsSync(candidate)) {
            return candidate;
        }
    }
    return 'vilan-lsp';
}

/// A clear, actionable error when the server can't be launched — instead of the
/// raw `spawn vilan-lsp ENOENT` buried in the output channel.
function reportMissingServer(command: string): void {
    const message =
        `Vilan: couldn't start the language server (\`${command}\`). ` +
        'Build it with `cargo build --release -p vilan-lsp`, then set ' +
        '`vilan.server.path` to the binary (e.g. `<repo>/target/release/vilan-lsp`) ' +
        'or put `vilan-lsp` on your PATH.';
    window.showErrorMessage(message, 'Open Settings').then((choice) => {
        if (choice === 'Open Settings') {
            commands.executeCommand('workbench.action.openSettings', 'vilan.server.path');
        }
    });
}

export function activate(context: ExtensionContext): void {
    const config = workspace.getConfiguration('vilan');
    const command = resolveServerPath(context, config.get<string>('server.path') || 'vilan-lsp');
    const stdPath = config.get<string>('stdPath') || '';

    // A configured/in-repo path that doesn't exist is a clear misconfiguration —
    // report it up front rather than letting the spawn fail opaquely. (A bare
    // `vilan-lsp` is a PATH lookup, so it's checked by the `start()` failure.)
    if (path.isAbsolute(command) && !fs.existsSync(command)) {
        reportMissingServer(command);
        return;
    }

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

    client = new LanguageClient('vilan', 'Vilan Language Server', serverOptions, clientOptions);
    client.start().catch((error) => {
        reportMissingServer(command);
        console.error('vilan-lsp failed to start:', error);
    });
}

export function deactivate(): Thenable<void> | undefined {
    return client?.stop();
}
