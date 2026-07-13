import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { workspace, window, commands, OutputChannel, ExtensionContext } from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;
let outputChannel: OutputChannel | undefined;

/// Resolve the language-server binary. An explicit `vilan.server.path` setting
/// wins; otherwise look for a binary built in-repo (the extension lives at
/// `<repo>/editors/vscode`, so the cargo target dir is two levels up), one
/// installed by `cargo install` (`~/.cargo/bin`), or the release toolchain the
/// install script manages (`~/.vilan/bin` — kept current by `vilan upgrade`),
/// and fall back to `vilan-lsp` on PATH. Developer builds outrank the release
/// install on purpose.
function resolveServerPath(context: ExtensionContext, configured: string): string {
    if (configured && configured !== 'vilan-lsp') {
        return configured;
    }
    const repoRoot = path.resolve(context.extensionPath, '..', '..');
    const candidates = [
        path.join(repoRoot, 'target', 'release', 'vilan-lsp'),
        path.join(repoRoot, 'target', 'debug', 'vilan-lsp'),
        path.join(os.homedir(), '.cargo', 'bin', 'vilan-lsp'),
        path.join(os.homedir(), '.vilan', 'bin', 'vilan-lsp'),
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
        'Install the toolchain (the install script puts `vilan-lsp` in `~/.vilan/bin`), ' +
        'or build it with `cargo build --release -p vilan-lsp` and set ' +
        '`vilan.server.path` to the binary — or put `vilan-lsp` on your PATH.';
    window.showErrorMessage(message, 'Open Settings').then((choice) => {
        if (choice === 'Open Settings') {
            commands.executeCommand('workbench.action.openSettings', 'vilan.server.path');
        }
    });
}

/// (Re)start the language client from current settings — used on activation and
/// by the `vilan.restartServer` command, so a rebuilt server (or a changed
/// `vilan.server.path` / `vilan.stdPath`) is picked up without reloading the
/// window. Replaces any running client; reuses one output channel.
async function startClient(context: ExtensionContext): Promise<void> {
    if (client) {
        await client.stop().catch(() => undefined);
        client = undefined;
    }

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
        outputChannel,
    };

    client = new LanguageClient('vilan', 'Vilan Language Server', serverOptions, clientOptions);
    try {
        await client.start();
    } catch (error) {
        client = undefined;
        reportMissingServer(command);
        console.error('vilan-lsp failed to start:', error);
    }
}

export function activate(context: ExtensionContext): void {
    outputChannel = window.createOutputChannel('Vilan Language Server');
    context.subscriptions.push(outputChannel);

    void startClient(context);

    context.subscriptions.push(
        commands.registerCommand('vilan.restartServer', async () => {
            await startClient(context);
            if (client) {
                window.showInformationMessage('Vilan: language server restarted.');
            }
        }),
    );
}

export function deactivate(): Thenable<void> | undefined {
    return client?.stop();
}
