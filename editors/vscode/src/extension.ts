import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
    workspace,
    window,
    commands,
    CodeAction,
    CodeActionKind,
    OutputChannel,
    ExtensionContext,
    Range,
    TextDocument,
    TextEdit,
} from 'vscode';
import {
    DidChangeConfigurationNotification,
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;
let outputChannel: OutputChannel | undefined;

/// The feature settings the server reads (gating inlay hints / semantic tokens,
/// and — for WO-3 — completion's call style). Sent as `initializationOptions` at
/// startup and re-sent via `workspace/didChangeConfiguration` on change. Shaped
/// as the server's `Config::from_settings` expects. (`organizeImports.onSave` is
/// handled entirely on the client, so it is not included here.)
function readFeatureConfig(): object {
    const config = workspace.getConfiguration('vilan');
    return {
        inlayHints: { enabled: config.get<boolean>('inlayHints.enabled', true) },
        semanticTokens: { enabled: config.get<boolean>('semanticTokens.enabled', true) },
        completion: { functionCall: config.get<string>('completion.functionCall', 'full') },
    };
}

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
        // Seed the server's feature settings; later changes go via
        // `workspace/didChangeConfiguration` (see `activate`).
        initializationOptions: readFeatureConfig(),
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

/// The Organize Imports text edits the server offers for `document`, or `[]`.
/// Backs `vilan.organizeImports.onSave` by requesting the server's OWN
/// `source.organizeImports` action — so on-save organizing is byte-identical to
/// invoking it from the Source Action menu (the "editor and fmt can never
/// disagree" chain extends to the save hook).
async function organizeImportsEdits(document: TextDocument): Promise<TextEdit[]> {
    if (!client) {
        return [];
    }
    const wholeFile = new Range(0, 0, document.lineCount, 0);
    const actions = await commands.executeCommand<CodeAction[]>(
        'vscode.executeCodeActionProvider',
        document.uri,
        wholeFile,
        CodeActionKind.SourceOrganizeImports.value,
    );
    const organize = actions?.find((action) =>
        action.kind?.contains(CodeActionKind.SourceOrganizeImports),
    );
    return organize?.edit?.get(document.uri) ?? [];
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

    // Live setting changes. A server-path / std-path change needs a restart to
    // take effect; a feature toggle is pushed to the running server, which reads
    // its config per request (no re-registration needed).
    context.subscriptions.push(
        workspace.onDidChangeConfiguration(async (event) => {
            if (
                event.affectsConfiguration('vilan.server.path') ||
                event.affectsConfiguration('vilan.stdPath')
            ) {
                await startClient(context);
                return;
            }
            if (
                client &&
                (event.affectsConfiguration('vilan.inlayHints') ||
                    event.affectsConfiguration('vilan.semanticTokens') ||
                    event.affectsConfiguration('vilan.completion'))
            ) {
                client.sendNotification(DidChangeConfigurationNotification.type, {
                    settings: { vilan: readFeatureConfig() },
                });
            }
        }),
    );

    // `vilan.organizeImports.onSave`: run the server's Organize Imports action
    // before a save writes the file. This is the extension's own hook rather
    // than mutating the user's `editor.codeActionsOnSave` — it leaves that config
    // untouched (respecting it), and because organizing is a fixed point, a user
    // who has ALSO listed `source.organizeImports` there gets no double effect.
    context.subscriptions.push(
        workspace.onWillSaveTextDocument((event) => {
            if (event.document.languageId !== 'vilan' || !client) {
                return;
            }
            const enabled = workspace
                .getConfiguration('vilan', event.document)
                .get<boolean>('organizeImports.onSave', false);
            if (enabled) {
                event.waitUntil(organizeImportsEdits(event.document));
            }
        }),
    );
}

export function deactivate(): Thenable<void> | undefined {
    return client?.stop();
}
