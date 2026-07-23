use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::{Child, ExitCode},
    time::{Duration, SystemTime},
};

use ariadne::{Color, Label, Report, ReportKind, sources};
use clap::{Parser as _, Subcommand};
mod hmr;
mod upgrade;

use vilan_core::analyzer::{analyze, check_library_contract};
use vilan_core::async_infer;
use vilan_core::call_graph::CallGraph;
use vilan_core::context;
use vilan_core::manifest::Package;
use vilan_core::transformer::transform;
use vilan_core::{Backend, BuildOptions, Manifest, Platform, Workspace};

/// The vilan language toolchain.
#[derive(clap::Parser)]
#[command(
    name = "vilan",
    version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("VILAN_BUILD_SHA"), ")"),
    about
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compile to JavaScript, writing `<file>.js`. With no path, compiles the
    /// project entry from the nearest `vilan.toml`.
    Build {
        /// A `.vl` file, a project directory, or omitted to use `vilan.toml`.
        file: Option<PathBuf>,
        /// Print the JavaScript to stdout instead of writing `<file>.js`.
        #[arg(long)]
        stdout: bool,
        /// The platform to build for: `node` (`node:24`), `deno` (`deno:2`), `bun`
        /// (`bun:1`), `browser`, or `none`. Overrides the package's `target`; defaults
        /// to it, else `node`. `--target` is an accepted alias.
        #[arg(long, alias = "target")]
        platform: Option<String>,
        /// The emitter backend: `js` (the only backend today).
        #[arg(long)]
        backend: Option<String>,
        /// Also emit `.parse.out` / `.analyze.out` / `.callgraph.out` debug dumps.
        #[arg(short, long)]
        debug: bool,
        /// Rebuild whenever a watched `.vl` source file changes (Ctrl-C to stop).
        #[arg(long)]
        watch: bool,
    },
    /// Type-check, reporting diagnostics without writing output. With no path,
    /// checks the project entry from the nearest `vilan.toml`.
    Check {
        /// A `.vl` file, a project directory, or omitted to use `vilan.toml`.
        file: Option<PathBuf>,
        /// The platform to check for: `node` (`node:24`), `deno` (`deno:2`), `bun`
        /// (`bun:1`), `browser`, or `none`. Overrides the package's `target`; defaults
        /// to it, else `node`. `--target` is an accepted alias.
        #[arg(long, alias = "target")]
        platform: Option<String>,
        /// The emitter backend: `js` (the only backend today).
        #[arg(long)]
        backend: Option<String>,
        /// Also emit `.parse.out` / `.analyze.out` / `.callgraph.out` debug dumps.
        #[arg(short, long)]
        debug: bool,
        /// Re-check whenever a watched `.vl` source file changes (Ctrl-C to stop).
        #[arg(long)]
        watch: bool,
    },
    /// Build and run a source file, forwarding any trailing arguments to the
    /// program (reach them with `process::args()`).
    Run {
        /// A `.vl` file, a project directory, or omitted to use `vilan.toml`.
        file: Option<PathBuf>,
        /// Rebuild and restart whenever a watched `.vl` source file changes. Place it
        /// before the file (`vilan run --watch app.vl`), ahead of any program args.
        #[arg(long)]
        watch: bool,
        /// Turn off hot module replacement under `--watch` (plain restart-the-server
        /// behavior). HMR is otherwise on for a workspace with a browser leg.
        #[arg(long)]
        no_hmr: bool,
        /// The `127.0.0.1` port for the HMR dev channel (`0` ⇒ an OS-assigned
        /// ephemeral port). Only meaningful with `--watch` on an HMR-eligible project.
        #[arg(long, default_value_t = hmr::DEFAULT_HMR_PORT)]
        hmr_port: u16,
        /// In a workspace with more than one `node` package, which one to run (by
        /// package name). The others still compile as part of the workspace but
        /// are not launched. Unnecessary for a single-node workspace.
        #[arg(long)]
        entry: Option<String>,
        /// Arguments passed through to the running program (after the file).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Format vilan source files in place. Already-formatted (and any not-yet
    /// formattable) files are left untouched.
    Fmt {
        /// Files or directories to format. Defaults to the current directory.
        paths: Vec<PathBuf>,
        /// Report files that would change without rewriting them (exit 1 if any).
        #[arg(long)]
        check: bool,
    },
    /// Run `*_test.vl` tests (each passes by exiting 0; a failed `assert` panics).
    Test {
        /// A test file, a directory of tests, or omitted to use the project root.
        path: Option<PathBuf>,
        /// Re-run the tests whenever a watched `.vl` source file changes (Ctrl-C to stop).
        #[arg(long)]
        watch: bool,
    },
    /// Update this binary (and `vilan-lsp` beside it) to the newest release.
    /// This is the only command that touches the network.
    Upgrade {
        /// Report whether a newer release exists without changing anything.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    // Compilation recurses over deeply-nested ASTs and type graphs (e.g. closures
    // stored in data structures plus generic monomorphization), which can run
    // past the default main-thread stack on otherwise-valid programs. Do the work
    // on a worker with a generous stack, as rustc and other compilers do; the
    // reservation is virtual address space, so it costs nothing unless used.
    const COMPILER_STACK_SIZE: usize = 256 * 1024 * 1024;
    std::thread::Builder::new()
        .stack_size(COMPILER_STACK_SIZE)
        .spawn(run_cli)
        .expect("spawn compiler thread")
        .join()
        .expect("compiler thread panicked")
}

fn run_cli() -> ExitCode {
    match Cli::parse().command {
        Command::Build {
            file,
            stdout,
            platform,
            backend,
            debug,
            watch,
        } => match effective_backend(backend.as_deref()) {
            Err(message) => report_error(&message),
            Ok(_backend) => {
                let roots = watch.then(|| watch_roots(&file));
                run_or_watch(roots, move || {
                    build_once(file.clone(), stdout, platform.clone(), debug)
                })
            }
        },
        Command::Check {
            file,
            platform,
            backend,
            debug,
            watch,
        } => match effective_backend(backend.as_deref()) {
            Err(message) => report_error(&message),
            Ok(_backend) => {
                let roots = watch.then(|| watch_roots(&file));
                run_or_watch(roots, move || {
                    check_once(file.clone(), platform.clone(), debug)
                })
            }
        },
        // `run`/`test` execute with `node`. `run --watch` restarts the process on a
        // change (see `run_watch`); the others just re-run the command.
        Command::Run {
            file,
            args,
            watch,
            no_hmr,
            hmr_port,
            entry,
        } => {
            if watch {
                run_watch(file, args, no_hmr, hmr_port, entry)
            } else {
                run_once(file, &args, entry.as_deref())
            }
        }
        Command::Test { path, watch } => {
            let roots = watch.then(|| watch_roots(&path));
            run_or_watch(roots, move || test(path.clone()))
        }
        Command::Fmt { paths, check } => fmt(&paths, check),
        Command::Upgrade { check } => upgrade::upgrade(check),
    }
}

/// Builds the project once (a lone package / bare file for its `--platform`, a
/// workspace for each member's platform; a `[library]` isn't buildable).
fn build_once(
    file: Option<PathBuf>,
    stdout: bool,
    platform: Option<String>,
    debug: bool,
) -> ExitCode {
    with_project(file, |project| match project {
        Project::Single {
            unit,
            platform: package_platform,
        } => match effective_platform(platform.as_deref(), package_platform) {
            Ok(Platform::None) => no_host_platform(),
            Ok(platform) => build_single(&unit, stdout, platform, debug),
            Err(message) => report_error(&message),
        },
        // A workspace builds each member for its own declared platform, so the
        // `--platform` flag doesn't apply.
        Project::Workspace { root, members } => build_workspace(&root, &members, debug),
        Project::Library { name, .. } => not_buildable_library(&name),
    })
}

/// Type-checks the project once. A standalone `[library]` has no fixed platform, so
/// it verifies the platform contract (§4.2) instead of a single-platform build.
fn check_once(file: Option<PathBuf>, platform: Option<String>, debug: bool) -> ExitCode {
    with_project(file, |project| match project {
        Project::Single {
            unit,
            platform: package_platform,
        } => match effective_platform(platform.as_deref(), package_platform) {
            // A `none` package is a pure library — not buildable, but type-checkable
            // (against the base layer only).
            Ok(platform) => check_single(&unit, platform, debug),
            Err(message) => report_error(&message),
        },
        Project::Workspace { members, .. } => check_workspace(&members, debug),
        Project::Library { dir, name } => check_library(&dir, &name),
    })
}

/// Builds and runs the project once with Node, waiting for it to exit and
/// propagating its code (the blocking, non-`--watch` path). `entry` picks the
/// Node leg to run in a multi-node workspace (A15).
fn run_once(file: Option<PathBuf>, args: &[String], entry: Option<&str>) -> ExitCode {
    with_project(file, |project| match project {
        Project::Single { unit, platform } => {
            let platform = platform.unwrap_or_default();
            if matches!(platform, Platform::Node { .. }) {
                run_single(&unit, args)
            } else {
                eprintln!(
                    "error: `vilan run` executes with Node, but the package platform is `{}`",
                    platform.name()
                );
                ExitCode::FAILURE
            }
        }
        Project::Workspace { root, members } => run_workspace(&root, &members, args, entry),
        Project::Library { name, .. } => not_buildable_library(&name),
    })
}

// --- `--watch` mode (roadmap P5) --------------------------------------------

/// How often the watcher polls for changes.
const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(300);

/// Runs `action` once and returns its exit code (no `--watch`, `roots` is `None`),
/// or — under `--watch` — re-runs it on every change to a `.vl` file under `roots`.
fn run_or_watch(roots: Option<Vec<PathBuf>>, mut action: impl FnMut() -> ExitCode) -> ExitCode {
    match roots {
        None => action(),
        Some(roots) => watch_loop(&roots, move || {
            let _ = action();
        }),
    }
}

/// The directories to watch, from a command's path argument: an explicit directory
/// as-is (a workspace root covers every member); a file's parent (its `pkg::`
/// siblings); with no path, the nearest project root, else the working directory.
fn watch_roots(file: &Option<PathBuf>) -> Vec<PathBuf> {
    let root = match file {
        Some(path) if path.is_dir() => path.clone(),
        Some(path) => pkg_root_of(path),
        None => env::current_dir()
            .ok()
            .map(|cwd| find_project_root(&cwd).unwrap_or(cwd))
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    vec![root]
}

/// A snapshot of every `.vl` file under `roots` (recursively) → its last-modified
/// time. Only `.vl` files are tracked, so the compiler's own `.js` / `dist` / `.out`
/// output can never trigger a rebuild; comparing two snapshots detects edits,
/// additions, and removals.
fn scan_vl(roots: &[PathBuf]) -> BTreeMap<PathBuf, SystemTime> {
    let mut files = Vec::new();
    for root in roots {
        collect_vl_files(root, &mut files);
    }
    files
        .into_iter()
        .filter_map(|path| {
            let modified = fs::metadata(&path).and_then(|meta| meta.modified()).ok()?;
            Some((path, modified))
        })
        .collect()
}

/// Runs `action`, then re-runs it whenever a watched `.vl` file changes — polling
/// every [`WATCH_POLL_INTERVAL`]. Returns only when there's nothing to watch;
/// otherwise loops until `Ctrl-C` (which, via the shared terminal process group,
/// also stops any `run --watch` child).
fn watch_loop(roots: &[PathBuf], mut action: impl FnMut()) -> ExitCode {
    if roots.iter().all(|root| !root.exists()) {
        eprintln!("error: nothing to watch (no such path)");
        return ExitCode::FAILURE;
    }
    let watched = roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("[watch] watching {watched} for `.vl` changes — Ctrl-C to stop");
    action();
    let mut snapshot = scan_vl(roots);
    loop {
        std::thread::sleep(WATCH_POLL_INTERVAL);
        let next = scan_vl(roots);
        if next != snapshot {
            snapshot = next;
            eprintln!("\n[watch] change detected — re-running");
            action();
        }
    }
}

/// `vilan run --watch`: rebuild and restart the program on every change. Each round
/// stops the previous process first (so a server frees its port), then spawns the
/// new one without waiting and holds its handle for the next round.
///
/// When the project is a workspace with a browser leg and `--no-hmr` isn't set,
/// hot module replacement is active (hmr.md §1): a dev channel serves the browser,
/// and each round classifies the rebuilt bytes (hmr.md §6) — restarting the Node
/// child only when the server bundle changed, and pushing `swap` / `css` / `error`
/// to the browser instead of bouncing it. Otherwise this is the plain
/// restart-the-server loop, byte-for-byte as before.
fn run_watch(
    file: Option<PathBuf>,
    args: Vec<String>,
    no_hmr: bool,
    hmr_port: u16,
    entry: Option<String>,
) -> ExitCode {
    let roots = watch_roots(&file);
    let mut child: Option<Child> = None;
    let channel = if no_hmr {
        None
    } else {
        activate_hmr(&file, hmr_port)
    };
    let mut state = WatchState::default();
    watch_loop(&roots, move || match &channel {
        Some(channel) => {
            child = hmr_round(
                channel,
                file.clone(),
                &args,
                &mut state,
                child.take(),
                entry.as_deref(),
            );
        }
        None => {
            // The plain restart loop recompiles and respawns wholesale, so the
            // per-leg skip doesn't drop in naturally here (there are no retained
            // per-leg artifacts to reuse) (backlog E12).
            if let Some(mut previous) = child.take() {
                let _ = previous.kill();
                let _ = previous.wait();
            }
            child = build_and_spawn_run(file.clone(), &args, entry.as_deref());
        }
    })
}

/// The carried-over state of an HMR `run --watch` across rounds (backlog E12):
/// the previous good artifacts (with their source sets) for the byte classifier
/// and the per-leg skip, plus the two guards that force a full recompile
/// regardless of the changed set.
#[derive(Default)]
struct WatchState {
    /// Each host leg's last good artifact — the classifier's `previous`, and the
    /// source of the reused bytes when a leg is skipped.
    legs: Vec<hmr::LegArtifact>,
    /// The previous round failed to compile: no leg has a trustworthy artifact
    /// to reuse, so recompile every leg until a round succeeds.
    failed: bool,
    /// A fingerprint of every `vilan.toml` under the watch root (workspace,
    /// members, and in-tree dependencies alike). A manifest can change a leg's
    /// output without touching its `.vl` sources (a dependency, a platform, a
    /// build option), so a change here forces a full recompile. `None` until
    /// the first round establishes it.
    manifest: Option<u64>,
}

/// Turns HMR on for `run --watch` when the project is a workspace with at least
/// one browser leg (hmr.md §1). Binds the dev channel on `127.0.0.1:port`
/// (`port` `0` ⇒ ephemeral) and announces it. A port already in use is a warning,
/// not a crash — the watch continues without HMR. `None` (silently) when the
/// project isn't HMR-eligible.
fn activate_hmr(file: &Option<PathBuf>, port: u16) -> Option<hmr::DevChannel> {
    let project = resolve_project(file.clone()).ok()?;
    let Project::Workspace { root, members } = &project else {
        return None;
    };
    if !members
        .iter()
        .any(|(_, platform)| matches!(platform, Platform::Browser))
    {
        return None;
    }
    match hmr::DevChannel::bind(port, root.join("dist")) {
        Ok(channel) => {
            println!("hmr: dev channel on 127.0.0.1:{}", channel.port());
            Some(channel)
        }
        Err(error) => {
            eprintln!(
                "warning: HMR dev channel could not bind 127.0.0.1:{port} ({error}); \
                 continuing to watch without HMR"
            );
            None
        }
    }
}

/// One HMR watch round (hmr.md §6): rebuild every host leg, classify the raw
/// bundle bytes against the previous round, write `dist/` (browser legs get the
/// shim prepended, with this build's version embedded), restart the Node child
/// only when the server bundle changed, and push the round event to the browser.
/// A compile failure pushes an `error` event and keeps the last good build —
/// the standard HMR contract — leaving `previous` and the running `child` intact.
fn hmr_round(
    channel: &hmr::DevChannel,
    file: Option<PathBuf>,
    args: &[String],
    state: &mut WatchState,
    child: Option<Child>,
    entry: Option<&str>,
) -> Option<Child> {
    let (root, members) = match resolve_project(file) {
        Ok(Project::Workspace { root, members }) => (root, members),
        // The project stopped being an HMR-eligible workspace (a manifest edit,
        // say). Report it as a failed round: overlay + keep the last good build.
        Ok(_) | Err(_) => {
            eprintln!("error: the HMR project is no longer a runnable workspace");
            state.failed = true;
            channel.push("error", Some("build failed — see the terminal"));
            return child;
        }
    };

    // Decide which legs this round may SKIP — reuse the previous artifact for
    // rather than recompile (backlog E12, half b). Reuse is decided by CONTENT,
    // never by mtime: a leg qualifies only when every source its artifact was
    // compiled from re-hashes, right now, to the hash it was compiled with
    // (mtime merely *triggers* rounds — review finding, 2026-07-21). The safe
    // default (skip nothing) covers the first round, a prior failure, and a
    // manifest change; a deleted or unreadable source fails its re-hash and
    // recompiles by construction.
    let manifest = manifest_fingerprint(&root);
    let manifest_changed = state.manifest.is_some_and(|previous| previous != manifest);
    state.manifest = Some(manifest);
    let force_full = hmr::round_forces_full(state.legs.is_empty(), state.failed, manifest_changed);
    let current_hash = |path: &Path| -> Option<u64> {
        fs::read_to_string(path)
            .ok()
            .map(|text| vilan_core::content_hash(&text))
    };
    let skip: BTreeSet<String> = if force_full {
        BTreeSet::new()
    } else {
        members
            .iter()
            .filter(|(_, platform)| !platform.is_none())
            .filter_map(|(unit, _)| {
                let previous = state.legs.iter().find(|leg| leg.name == unit.name)?;
                hmr::leg_is_current(&previous.sources, &current_hash).then(|| unit.name.clone())
            })
            .collect()
    };

    // Compile every host leg (skipped legs excepted), capturing the RAW bundle
    // bytes (before the shim is prepended — the shim embeds the version, so
    // shim-inclusive bytes would differ every round and misclassify everything
    // as a swap).
    let mut next = Vec::new();
    let mut other_assets: Vec<(String, String, String)> = Vec::new();
    for (unit, platform) in &members {
        if platform.is_none() {
            continue;
        }
        if skip.contains(&unit.name) {
            // Reuse the previous artifact verbatim: the leg's sources are
            // unchanged, so a recompile would reproduce these exact bytes (the
            // classifier then sees no change and pushes nothing — identical to
            // having recompiled). Its non-css assets are already on disk from the
            // round that built them, so they need no rewrite.
            let prior = state
                .legs
                .iter()
                .find(|leg| leg.name == unit.name)
                .expect("skippable_legs only skips a leg with a previous artifact");
            println!("hmr: skipped {} (sources unchanged)", unit.name);
            next.push(prior.clone());
            continue;
        }
        let mut overlay_text = String::new();
        let (javascript, assets, sources) = match compile_unit(
            unit,
            *platform,
            false,
            matches!(platform, Platform::Browser),
            Some(&mut overlay_text),
        ) {
            Ok(compiled) => compiled,
            // `compile_unit` has already reported the diagnostics to the
            // terminal (unchanged); `overlay_text` is the SAME diagnostics
            // rendered ANSI-free for the in-page overlay (hmr.md §§2/§6, the S1
            // residue closed). Keep the last good build.
            Err(_) => {
                state.failed = true;
                let message = if overlay_text.is_empty() {
                    "build failed — see the terminal"
                } else {
                    overlay_text.as_str()
                };
                channel.push("error", Some(message));
                return child;
            }
        };
        let mut assembled = vilan_core::const_eval::assemble_assets(&assets);
        let css = assembled
            .remove("css")
            .filter(|content| !content.is_empty());
        // Any non-css asset kind still lands on disk each round, exactly as
        // `write_assets` would put it (uniform with the build/run paths); it
        // just doesn't participate in classification — css is the only kind
        // the dev runtime knows how to hot-swap.
        for (kind, content) in assembled {
            other_assets.push((unit.name.clone(), kind, content));
        }
        next.push(hmr::LegArtifact {
            name: unit.name.clone(),
            is_browser: matches!(platform, Platform::Browser),
            bundle: javascript,
            css,
            sources: sources.into_iter().collect(),
        });
    }

    // The ONE Node leg this watch runs (A15): `--entry` picks it in a multi-node
    // workspace, a lone node leg is picked automatically, a browser-only workspace
    // has none. The non-selected node legs compiled above (they are part of the
    // workspace) but are never launched, and — since they are not run and not
    // served — a change to one of them drives no restart (the classifier keys the
    // restart on the SELECTED leg only). An ambiguous choice is reported below,
    // when a restart is actually attempted.
    let selection = select_node_entry(&members, entry);
    let server_leg = match &selection {
        Ok(Some(unit)) => Some(unit.name.as_str()),
        _ => None,
    };
    let decision = hmr::classify(&state.legs, &next, server_leg);
    if decision.bump_version {
        channel.bump_version();
    }
    let version = channel.version();

    // Write `dist/` from the freshly-compiled legs: browser bundles carry the
    // shim (with the current port + version embedded) so every served browser
    // bundle's version matches what the channel reports on connect; node bundles
    // and CSS sidecars are written verbatim.
    let dist = root.join("dist");
    if let Err(error) = fs::create_dir_all(&dist) {
        eprintln!("error: cannot create {}: {error}", dist.display());
        state.failed = true;
        channel.push("error", Some("build failed — see the terminal"));
        return child;
    }
    for leg in &next {
        let bundle_path = dist.join(format!("{}.js", leg.name));
        let contents = if leg.is_browser {
            hmr::instrument(&leg.bundle, channel.port(), version, &leg.name)
        } else {
            leg.bundle.clone()
        };
        if let Err(error) = fs::write(&bundle_path, contents) {
            eprintln!("error: cannot write {}: {error}", bundle_path.display());
        }
        if let Some(css) = &leg.css {
            let css_path = dist.join(format!("{}.css", leg.name));
            if let Err(error) = fs::write(&css_path, css) {
                eprintln!("error: cannot write {}: {error}", css_path.display());
            }
        }
    }
    for (name, kind, content) in &other_assets {
        let asset_path = dist.join(format!("{name}.{kind}"));
        if let Err(error) = fs::write(&asset_path, content) {
            eprintln!("error: cannot write {}: {error}", asset_path.display());
        }
    }

    state.legs = next;
    // This round completed: clear the failure guard so the next round may skip
    // again (the previous-failure force-full no longer applies).
    state.failed = false;

    // Restart the Node child only when the server bundle changed (or on the
    // first round, to spawn it). A client-only or CSS-only round leaves the
    // server running and its port warm.
    let mut child = child;
    if decision.restart_server {
        if let Some(mut running) = child.take() {
            let _ = running.kill();
            let _ = running.wait();
        }
        child = match &selection {
            Ok(Some(unit)) => {
                // Run from the workspace root so the server reads sibling
                // `dist/*.js`, exactly as `run_workspace` / `build_and_spawn_run`.
                let script = Path::new("dist").join(format!("{}.js", unit.name));
                match spawn_node(&script, args, Some(&root)) {
                    Ok(spawned) => Some(spawned),
                    Err(error) => {
                        eprintln!("error: failed to launch `node`: {error}");
                        None
                    }
                }
            }
            // No node leg at all: HMR still serves the browser leg(s).
            Ok(None) => None,
            // 2+ node legs and no `--entry` (or a bad `--entry`): report it (once,
            // on the first round's spawn attempt) and serve the browser anyway.
            Err(message) => {
                eprintln!("error: {message}");
                None
            }
        };
    }

    match &decision.push {
        Some(hmr::Push::Swap) => channel.push("swap", None),
        Some(hmr::Push::Css(assets)) => {
            for asset in assets {
                channel.push_css(asset);
            }
        }
        None => {}
    }

    child
}

/// A fingerprint of the workspace + member `vilan.toml` files (backlog E12). A
/// change here — a dependency, a platform, a build option — can alter a leg's
/// output without touching any `.vl` source, so a differing fingerprint forces
/// the round to recompile every leg rather than skip. Walks the watch root for
/// **every** `vilan.toml` (workspace, members, and in-tree dependency packages
/// alike — a dependency's manifest changes its dependents' output too) and
/// hashes each path + content (unreadable ⇒ `None`), so an added, removed, or
/// edited manifest all shift the value.
fn manifest_fingerprint(root: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    fn collect_manifests(directory: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_manifests(&path, found);
            } else if path.file_name().and_then(|name| name.to_str()) == Some("vilan.toml") {
                found.push(path);
            }
        }
    }
    let mut manifests = Vec::new();
    collect_manifests(root, &mut manifests);
    manifests.sort();
    let mut hasher = DefaultHasher::new();
    for path in manifests {
        path.hash(&mut hasher);
        fs::read(&path).ok().hash(&mut hasher);
    }
    hasher.finish()
}

/// Builds the run target and spawns it with Node **without waiting**, returning the
/// child so the next `run --watch` round can stop it. `None` after reporting a
/// compile error or a non-runnable project.
fn build_and_spawn_run(
    file: Option<PathBuf>,
    args: &[String],
    entry: Option<&str>,
) -> Option<Child> {
    let project = match resolve_project(file) {
        Ok(project) => project,
        Err(message) => {
            eprintln!("error: {message}");
            return None;
        }
    };
    let launch = |script: &Path, cwd: Option<&Path>| match spawn_node(script, args, cwd) {
        Ok(child) => Some(child),
        Err(error) => {
            eprintln!("error: failed to launch `node`: {error}");
            None
        }
    };
    match project {
        Project::Single { unit, platform } => {
            let platform = platform.unwrap_or_default();
            if !matches!(platform, Platform::Node { .. }) {
                eprintln!(
                    "error: `vilan run` executes with Node, but the package platform is `{}`",
                    platform.name()
                );
                return None;
            }
            let (javascript, assets, _sources) =
                compile_unit(&unit, Platform::default(), false, false, None).ok()?;
            // Assets go beside the *canonical* build output — `<entry>.css`, where
            // `build` writes them and the served program reads them — not beside the
            // /tmp watch script Node executes (which nothing serves). Each watch
            // round thus refreshes the on-disk sidecar for the dev loop (hmr.md §11
            // S0); the workspace arm below gets this for free via
            // `build_workspace_artifacts`.
            write_assets(&unit.entry.with_extension("js"), &assets);
            let script = env::temp_dir().join(format!("vilan-watch-{}.js", std::process::id()));
            if let Err(error) = fs::write(&script, javascript) {
                eprintln!("error: cannot write {}: {error}", script.display());
                return None;
            }
            launch(&script, None)
        }
        Project::Workspace { root, members } => {
            let server = match select_node_entry(&members, entry) {
                Ok(Some(unit)) => unit,
                Ok(None) => {
                    eprintln!("error: no `node` package in this workspace to run");
                    return None;
                }
                Err(message) => {
                    eprintln!("error: {message}");
                    return None;
                }
            };
            if build_workspace_artifacts(&root, &members, false).is_err() {
                return None;
            }
            launch(
                &Path::new("dist").join(format!("{}.js", server.name)),
                Some(&root),
            )
        }
        Project::Library { name, .. } => {
            not_buildable_library(&name);
            None
        }
    }
}

/// Prints an `error: <message>` line and returns the failure code.
fn report_error(message: &str) -> ExitCode {
    eprintln!("error: {message}");
    ExitCode::FAILURE
}

/// Reports that a `none`-platform package can't be built (it's a pure library).
fn no_host_platform() -> ExitCode {
    eprintln!(
        "error: the platform is `none` (a pure library); pick a host to build for with \
         `--platform node` or `--platform browser`"
    );
    ExitCode::FAILURE
}

/// Reports that a `[library]` can't be built or run on its own — it's compiled only
/// as a dependency of an app.
fn not_buildable_library(name: &str) -> ExitCode {
    eprintln!(
        "error: `{name}` is a `[library]`, built only as a dependency of an app, not on its own. \
         Verify its platform contract with `vilan check`, or build an app that depends on it."
    );
    ExitCode::FAILURE
}

/// Checks a standalone `[library]`: it has no fixed build platform, so instead of a
/// single-platform compile it verifies the **platform contract** (§4.2) — every
/// module's `pkg::` imports must resolve for every platform that module's layer
/// serves. Reports any violation; clean ⇒ success.
fn check_library(dir: &Path, name: &str) -> ExitCode {
    let spec = vilan_core::manifest::resolve_library(dir);
    let violations = check_library_contract(&spec);
    if violations.is_empty() {
        println!("{name}: platform contract OK");
        ExitCode::SUCCESS
    } else {
        for violation in &violations {
            eprintln!("error: {}", violation.msg);
        }
        ExitCode::FAILURE
    }
}

/// The effective build platform: an explicit `--platform`/`--target` flag wins (it
/// may name any platform, including `none`); otherwise the package's declared
/// `target`; otherwise the `node` default. `Err` carries a descriptive message for
/// an unrecognized or unsupported flag value.
fn effective_platform(flag: Option<&str>, package: Option<Platform>) -> Result<Platform, String> {
    match flag {
        Some(name) => Platform::parse(name),
        None => Ok(package.unwrap_or_default()),
    }
}

/// Validates a `--backend` flag value (only `js` today). The returned [`Backend`]
/// selects nothing yet — there's a single backend — so this exists to reject an
/// unknown name (e.g. `wasm`, not yet implemented) at the CLI boundary rather than
/// silently ignoring it.
fn effective_backend(flag: Option<&str>) -> Result<Backend, String> {
    match flag {
        Some(name) => {
            Backend::parse(name).ok_or_else(|| format!("unknown backend `{name}` (expected `js`)"))
        }
        None => Ok(Backend::default()),
    }
}

/// A package's source root from a bare entry file (no manifest): the entry's
/// parent directory, where its `import pkg::..` siblings live. Empty (a bare
/// filename) means the working directory.
fn pkg_root_of(entry: &Path) -> PathBuf {
    entry
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

/// Formats every `.vl` file under `paths` (a file, a directory walked
/// recursively, or the working directory when empty). In `--check` mode it only
/// reports files that would change; otherwise it rewrites them in place. The
/// formatter leaves a file untouched when it's already formatted or contains a
/// construct it can't yet print (it never produces non-round-tripping output).
fn fmt(paths: &[PathBuf], check: bool) -> ExitCode {
    let roots: Vec<PathBuf> = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    };
    let mut files = Vec::new();
    for root in &roots {
        collect_vl_files(root, &mut files);
    }
    let mut changed = 0;
    let mut failed = false;
    for file in &files {
        let source = match fs::read_to_string(file) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", file.display());
                failed = true;
                continue;
            }
        };
        let formatted = vilan_core::formatter::format(&source);
        if formatted == source {
            continue;
        }
        if check {
            println!("would reformat {}", file.display());
            changed += 1;
        } else if let Err(error) = fs::write(file, &formatted) {
            eprintln!("error: cannot write {}: {error}", file.display());
            failed = true;
        } else {
            println!("formatted {}", file.display());
        }
    }
    if failed || (check && changed > 0) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Collects every `.vl` file under `path` (recursing into directories), in a
/// stable (sorted) order.
fn collect_vl_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
            paths.sort();
            for entry in paths {
                collect_vl_files(&entry, out);
            }
        }
    } else if path.extension().and_then(|extension| extension.to_str()) == Some("vl") {
        out.push(path.to_path_buf());
    }
}

/// A buildable unit — a workspace member, a lone package, or a bare file: the
/// entry to compile, its package source root, the directory whose `vilan.toml`
/// declares its dependencies (for resolving the workspace), and its codegen
/// options. `name` labels a workspace member's `dist/<name>.js` output.
struct Unit {
    name: String,
    /// The entry file, resolved against the package root.
    entry: PathBuf,
    /// The package source root (where `import pkg::..` siblings resolve).
    pkg_root: PathBuf,
    /// The directory holding this unit's `vilan.toml` (from which its dependency
    /// workspace is resolved), or `None` for a bare file with no manifest.
    package_dir: Option<PathBuf>,
    options: BuildOptions,
}

/// A project to act on: a lone package / bare file (its platform chosen with the
/// `--platform` flag, defaulting to the package's), or a workspace of members each
/// built for its own fixed platform. The legacy `[server]`/`[client]` pair lowers
/// onto a two-member workspace.
enum Project {
    Single {
        unit: Unit,
        /// The package's declared `target` platform, if any (`None` ⇒ the `node`
        /// default).
        platform: Option<Platform>,
    },
    Workspace {
        root: PathBuf,
        members: Vec<(Unit, Platform)>,
    },
    /// A standalone `[library]`, addressed directly. Not a buildable app (a library
    /// is compiled only as a dependency), but `vilan check` verifies its platform
    /// contract. `dir` is the library's package directory; `name` labels diagnostics.
    Library { dir: PathBuf, name: String },
}

/// Resolves the project from an optional path, then runs `action`. An explicit
/// file is a single entry; a directory (or no path, via the working directory)
/// is read from its `vilan.toml`.
fn with_project(path: Option<PathBuf>, action: impl FnOnce(Project) -> ExitCode) -> ExitCode {
    match resolve_project(path) {
        Ok(project) => action(project),
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn resolve_project(path: Option<PathBuf>) -> Result<Project, String> {
    match path {
        // An explicit directory: the project rooted there.
        Some(path) if path.is_dir() => project_from_manifest(&path),
        // An explicit file (or a not-yet-existing path, so `compile` can report
        // the read error): a single entry, compiled directly with default options
        // (there's no manifest to read `[build]`/`target`/dependencies from).
        Some(path) => Ok(Project::Single {
            unit: Unit {
                name: String::new(),
                pkg_root: pkg_root_of(&path),
                entry: path,
                package_dir: None,
                options: BuildOptions::default(),
            },
            platform: None,
        }),
        // No path: find the enclosing project from the working directory.
        None => {
            let working_dir = env::current_dir()
                .map_err(|error| format!("cannot read the working directory: {error}"))?;
            let root = find_project_root(&working_dir).ok_or_else(|| {
                "no `vilan.toml` found here or in any parent directory; \
                 pass a source file to compile it directly"
                    .to_string()
            })?;
            project_from_manifest(&root)
        }
    }
}

/// Reads, parses, validates, and reports warnings for the `vilan.toml` in
/// `directory`.
fn read_manifest(directory: &Path) -> Result<Manifest, String> {
    let manifest_path = directory.join("vilan.toml");
    let contents = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let (manifest, warnings) = Manifest::parse(&contents)
        .map_err(|error| format!("invalid {}: {error}", manifest_path.display()))?;
    for warning in &warnings {
        eprintln!("warning: {} in {}", warning, manifest_path.display());
    }
    let errors = manifest.validate();
    if !errors.is_empty() {
        return Err(format!(
            "invalid {}:\n  - {}",
            manifest_path.display(),
            errors.join("\n  - ")
        ));
    }
    Ok(manifest)
}

/// Builds a [`Unit`] from a package manifest in `directory`.
fn unit_from_package(directory: &Path, package: &Package, options: BuildOptions) -> Unit {
    let pkg_root = directory.join(package.root());
    Unit {
        name: package.name.clone().unwrap_or_default(),
        entry: pkg_root.join(package.entry()),
        pkg_root,
        package_dir: Some(directory.to_path_buf()),
        options,
    }
}

/// The build units a `[package]` manifest contributes: one per `[entry.<name>]`
/// when declared (proposal/platform-coloring.md §4.2), else the single classic
/// unit. Entry units build browser-class entries FIRST (stable within a class)
/// — the order is semantic, so a process entry that serves bundles always
/// finds them freshly built, whatever order the manifest declares.
fn package_units(
    directory: &Path,
    package: &Package,
    manifest: &Manifest,
    options: BuildOptions,
) -> Vec<(Unit, Platform)> {
    if manifest.entries.is_empty() {
        let platform = package.resolved_target().unwrap_or_default();
        return vec![(unit_from_package(directory, package, options), platform)];
    }
    let pkg_root = directory.join(package.root());
    let mut units: Vec<(Unit, Platform)> = manifest
        .entries
        .iter()
        .map(|(name, entry)| {
            (
                Unit {
                    name: name.clone(),
                    entry: pkg_root.join(entry.path(name)),
                    pkg_root: pkg_root.clone(),
                    package_dir: Some(directory.to_path_buf()),
                    options,
                },
                entry.resolved_target().unwrap_or_default(),
            )
        })
        .collect();
    units.sort_by_key(|(_, platform)| !matches!(platform, Platform::Browser));
    units
}

/// Rejects two build units sharing a name — their `dist/<name>.js` outputs
/// would silently overwrite each other. (`none` members emit nothing, so they
/// can't collide.)
fn reject_output_collisions(members: &[(Unit, Platform)]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for (unit, platform) in members {
        if platform.is_none() {
            continue;
        }
        if !seen.insert(unit.name.as_str()) {
            return Err(format!(
                "two build units are both named `{}`, so their outputs would \
                 collide at dist/{}.js — rename one (the package name or the \
                 `[entry.<name>]`)",
                unit.name, unit.name
            ));
        }
    }
    Ok(())
}

/// Resolves the project rooted at `directory` from its `vilan.toml`. A `[package]`
/// is a single package (`entry` resolves against `root`; `target` is the default),
/// unless it declares `[entry.<name>]` sections — then it lowers onto a workspace
/// with one member per entry. A `[project]` is a workspace — each member builds
/// for its own platform (and may itself declare entries).
fn project_from_manifest(directory: &Path) -> Result<Project, String> {
    let manifest = read_manifest(directory)?;
    let options = manifest
        .build_options()
        .map_err(|error| format!("invalid {}/vilan.toml: {error}", directory.display()))?;

    // A workspace: each `[project] packages` member is built for its own platform.
    if let Some(project) = &manifest.project {
        let mut members = Vec::new();
        for member_path in &project.packages {
            let member_dir = directory.join(member_path);
            let member_manifest = read_manifest(&member_dir)?;
            // A `[library]` member is built only as a dependency of the apps that
            // import it, not on its own — skip it here. Only `[package]` (app)
            // members are buildable units.
            let Some(package) = member_manifest.package.as_ref() else {
                if member_manifest.library.is_some() {
                    continue;
                }
                return Err(format!(
                    "workspace member `{}` is not a `[package]` or `[library]`",
                    member_dir.display()
                ));
            };
            let member_options = member_manifest
                .build_options()
                .map_err(|error| format!("invalid {}/vilan.toml: {error}", member_dir.display()))?;
            members.extend(package_units(
                &member_dir,
                package,
                &member_manifest,
                member_options,
            ));
        }
        reject_output_collisions(&members)?;
        return Ok(Project::Workspace {
            root: directory.to_path_buf(),
            members,
        });
    }

    // A standalone `[library]` addressed directly: not a buildable app, but its
    // platform contract is checkable. (`[library]` workspace *members* are handled
    // above — skipped as build units; this is a library directory on its own.)
    if let Some(library) = &manifest.library {
        return Ok(Project::Library {
            dir: directory.to_path_buf(),
            name: library.name.clone().unwrap_or_default(),
        });
    }

    // A single package. `validate` guarantees one of the three sections is present,
    // and the others are ruled out above.
    let package = manifest.package.as_ref().expect("validated package");

    // `[entry.<name>]` sections: the single-package full-stack form
    // (proposal/platform-coloring.md §4.2). Lowers onto the same workspace
    // orchestration as a `[project]` — every entry builds to `dist/<name>.js`,
    // `run` picks the one node entry, `check` checks them all.
    if !manifest.entries.is_empty() {
        let members = package_units(directory, package, &manifest, options);
        reject_output_collisions(&members)?;
        return Ok(Project::Workspace {
            root: directory.to_path_buf(),
            members,
        });
    }

    Ok(Project::Single {
        unit: unit_from_package(directory, package, options),
        platform: package.resolved_target(),
    })
}

/// Resolves a unit's dependency workspace. A unit with no manifest (a bare file)
/// has no dependencies. Delegates to the shared `vilan_core::manifest::resolve_workspace`
/// so the CLI and LSP resolve identically. (The build platform isn't needed — the
/// graph is platform-independent; the analyzer reports any cross-platform import.)
fn resolve_workspace(unit: &Unit) -> Result<Workspace, String> {
    match &unit.package_dir {
        Some(package_dir) => vilan_core::manifest::resolve_workspace(package_dir),
        None => Ok(Workspace::default()),
    }
}

/// Resolves a unit's workspace and compiles its entry for `platform`, returning the
/// emitted JavaScript (or a failure code after reporting).
fn compile_unit(
    unit: &Unit,
    platform: Platform,
    emit_debug: bool,
    hmr: bool,
    overlay: Option<&mut String>,
) -> Result<(String, Vec<(String, String)>, Vec<(PathBuf, u64)>), ExitCode> {
    let workspace = match resolve_workspace(unit) {
        Ok(workspace) => workspace,
        Err(message) => {
            eprintln!("error: {message}");
            return Err(ExitCode::FAILURE);
        }
    };
    // HMR instrumentation is opt-in per compile (an HMR-active `run --watch`,
    // browser legs only) — every other caller passes `false`, so `build`/`run`/
    // `check` output stays byte-identical.
    let mut options = unit.options;
    options.hmr = hmr;
    compile_to_js(
        &unit.entry,
        &unit.pkg_root,
        platform,
        &options,
        &workspace,
        emit_debug,
        overlay,
    )
}

/// Builds a lone package / bare file, writing `<entry>.js` (or printing to stdout).
fn build_single(unit: &Unit, stdout: bool, platform: Platform, emit_debug: bool) -> ExitCode {
    let (javascript, assets, _sources) = match compile_unit(unit, platform, emit_debug, false, None)
    {
        Ok(compiled) => compiled,
        Err(code) => return code,
    };
    if stdout {
        print!("{javascript}");
        return ExitCode::SUCCESS;
    }
    let output_path = unit.entry.with_extension("js");
    write_assets(&output_path, &assets);
    match fs::write(&output_path, javascript) {
        Ok(()) => {
            println!(
                "Compiled {} -> {}",
                unit.entry.display(),
                output_path.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: cannot write {}: {error}", output_path.display());
            ExitCode::FAILURE
        }
    }
}

/// Type-checks a lone package / bare file, writing no output.
fn check_single(unit: &Unit, platform: Platform, emit_debug: bool) -> ExitCode {
    match compile_unit(unit, platform, emit_debug, false, None) {
        Ok(_) => {
            println!("{}: no errors", unit.entry.display());
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// Builds and runs a lone package's entry with Node, forwarding `args`.
fn run_single(unit: &Unit, args: &[String]) -> ExitCode {
    let (javascript, assets, _sources) =
        match compile_unit(unit, Platform::default(), false, false, None) {
            Ok(compiled) => compiled,
            Err(code) => return code,
        };
    // Const-eval assets (the CSS sidecar &c.) belong beside the *canonical* build
    // output — `<entry>.css`, where `build` writes them and a served page reads
    // them — not beside the temp script `run_node_script` hands Node, which the
    // program never reads. Same helper and placement as `build_single`, so `run`
    // keeps the on-disk sidecar fresh (const-eval.md §3; hmr.md §11 S0).
    write_assets(&unit.entry.with_extension("js"), &assets);
    run_node_script(&javascript, args)
}

/// Builds every host (non-`none`) member of a workspace into `<root>/dist/<name>.js`
/// — a `none` member is a pure library, compiled only as a dependency of a host.
/// Members build in declaration order (the client before the server, so the
/// server's `dist/client.js` exists). `--platform`/`--stdout` don't apply.
fn build_workspace(root: &Path, members: &[(Unit, Platform)], debug: bool) -> ExitCode {
    match build_workspace_artifacts(root, members, debug) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn build_workspace_artifacts(
    root: &Path,
    members: &[(Unit, Platform)],
    debug: bool,
) -> Result<(), ExitCode> {
    let dist = root.join("dist");
    if let Err(error) = fs::create_dir_all(&dist) {
        eprintln!("error: cannot create {}: {error}", dist.display());
        return Err(ExitCode::FAILURE);
    }
    for (unit, platform) in members {
        if platform.is_none() {
            continue;
        }
        let (javascript, assets, _sources) = compile_unit(unit, *platform, debug, false, None)?;
        let output = dist.join(format!("{}.js", unit.name));
        write_assets(&output, &assets);
        if let Err(error) = fs::write(&output, javascript) {
            eprintln!("error: cannot write {}: {error}", output.display());
            return Err(ExitCode::FAILURE);
        }
        println!("Compiled {} -> {}", unit.entry.display(), output.display());
    }
    Ok(())
}

/// Type-checks every member of a workspace (each for its own platform; a `none`
/// library against the base layer).
fn check_workspace(members: &[(Unit, Platform)], debug: bool) -> ExitCode {
    let mut ok = true;
    for (unit, platform) in members {
        ok &= compile_unit(unit, *platform, debug, false, None).is_ok();
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Selects the ONE Node leg a `run` executes from a workspace's members (A15).
/// An explicit `--entry <name>` picks it by package name; a lone Node leg is
/// picked automatically; a browser-only workspace has none (`Ok(None)`). The
/// non-selected Node legs are still compiled by the caller — they are part of
/// the workspace — but never launched. `Err` when the choice is ambiguous
/// (2+ Node legs, no `--entry`) or names a package that isn't a runnable Node
/// leg; the message lists the candidates and the flag.
fn select_node_entry<'members>(
    members: &'members [(Unit, Platform)],
    entry: Option<&str>,
) -> Result<Option<&'members Unit>, String> {
    let node_members: Vec<&Unit> = members
        .iter()
        .filter(|(_, platform)| matches!(platform, Platform::Node { .. }))
        .map(|(unit, _)| unit)
        .collect();
    match (entry, node_members.as_slice()) {
        (Some(name), _) => match node_members.iter().find(|unit| unit.name == name) {
            Some(unit) => Ok(Some(unit)),
            None => Err(no_such_node_entry(name, &node_members)),
        },
        (None, []) => Ok(None),
        (None, [unit]) => Ok(Some(unit)),
        (None, _) => Err(ambiguous_node_entry(&node_members)),
    }
}

/// The candidate package names, in workspace declaration order, for an entry
/// error message: `server, probe`.
fn node_entry_candidates(node_members: &[&Unit]) -> String {
    node_members
        .iter()
        .map(|unit| unit.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The message for a `run` on a multi-node workspace with no `--entry` (A15).
fn ambiguous_node_entry(node_members: &[&Unit]) -> String {
    format!(
        "this workspace has more than one `node` package to run — pick one with \
         --entry <name>: {}",
        node_entry_candidates(node_members)
    )
}

/// The message for `--entry <name>` naming a package that isn't a runnable Node
/// leg (A15).
fn no_such_node_entry(name: &str, node_members: &[&Unit]) -> String {
    if node_members.is_empty() {
        format!("no `node` package named `{name}` to run (this workspace runs no `node` package)")
    } else {
        format!(
            "no `node` package named `{name}` to run — candidates: {}",
            node_entry_candidates(node_members)
        )
    }
}

/// Builds a workspace, then runs its selected Node member (A15) with `node` from
/// the project root (so it can read sibling `dist/*.js`). `args` are forwarded.
fn run_workspace(
    root: &Path,
    members: &[(Unit, Platform)],
    args: &[String],
    entry: Option<&str>,
) -> ExitCode {
    let server = match select_node_entry(members, entry) {
        Ok(Some(unit)) => unit,
        Ok(None) => {
            eprintln!("error: no `node` package in this workspace to run");
            return ExitCode::FAILURE;
        }
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(code) = build_workspace_artifacts(root, members, false) {
        return code;
    }
    // Run from the project root so the server reads sibling `dist/*.js`; the script
    // path is relative to that working directory.
    let status = spawn_node(
        &Path::new("dist").join(format!("{}.js", server.name)),
        args,
        Some(root),
    )
    .and_then(|mut child| child.wait());
    exit_code_of(status)
}

/// Walks up from `start` for the nearest directory containing a `vilan.toml`.
fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut directory = start;
    loop {
        if directory.join("vilan.toml").is_file() {
            return Some(directory.to_path_buf());
        }
        directory = directory.parent()?;
    }
}

/// Runs the package's `*_test.vl` tests: each is compiled and executed, passing if
/// it exits 0 (a failed `assert` panics -> non-zero). Reports a pass/fail summary
/// and exits non-zero if any test fails.
fn test(path: Option<PathBuf>) -> ExitCode {
    let tests = match discover_tests(path) {
        Ok(tests) => tests,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    if tests.is_empty() {
        println!("no `*_test.vl` tests found");
        return ExitCode::SUCCESS;
    }
    println!("running {} test(s)", tests.len());
    let mut passed = 0u32;
    let mut failed = 0u32;
    for test in &tests {
        match run_test(test) {
            Ok(()) => {
                passed += 1;
                println!("  ok    {}", test.display());
            }
            Err(detail) => {
                failed += 1;
                println!("  FAIL  {}", test.display());
                for line in detail.lines() {
                    println!("        {line}");
                }
            }
        }
    }
    println!("\n{passed} passed, {failed} failed");
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Compiles and executes one test. `Ok` if it exits 0; otherwise `Err(detail)`
/// with the captured runtime output (empty for a compile error, which
/// `compile_to_js` has already reported to stderr).
fn run_test(file: &Path) -> Result<(), String> {
    let (javascript, _assets, _sources) = compile_to_js(
        file,
        &pkg_root_of(file),
        Platform::default(),
        &BuildOptions::default(),
        &Workspace::default(),
        false,
        None,
    )
    .map_err(|_| String::new())?;
    let script = env::temp_dir().join(format!("vilan-test-{}.js", std::process::id()));
    if let Err(error) = fs::write(&script, javascript) {
        return Err(format!("cannot write {}: {error}", script.display()));
    }
    let output = std::process::Command::new("node").arg(&script).output();
    let _ = fs::remove_file(&script);
    match output {
        Ok(result) if result.status.success() => Ok(()),
        Ok(result) => {
            let mut detail = String::from_utf8_lossy(&result.stdout).into_owned();
            detail.push_str(&String::from_utf8_lossy(&result.stderr));
            Err(detail.trim_end().to_string())
        }
        Err(error) => Err(format!("failed to launch `node`: {error}")),
    }
}

/// The `*_test.vl` files to run: a single file, the test files directly in a given
/// directory, or — with no path — those in the project root (nearest `vilan.toml`).
fn discover_tests(path: Option<PathBuf>) -> Result<Vec<PathBuf>, String> {
    let directory = match path {
        Some(path) if path.is_file() => return Ok(vec![path]),
        Some(path) if path.is_dir() => path,
        Some(path) => return Err(format!("{} does not exist", path.display())),
        None => {
            let working_dir = env::current_dir()
                .map_err(|error| format!("cannot read the working directory: {error}"))?;
            find_project_root(&working_dir).ok_or_else(|| {
                "no `vilan.toml` found here or in any parent directory; \
                 pass a test file or directory"
                    .to_string()
            })?
        }
    };
    let mut tests: Vec<PathBuf> = fs::read_dir(&directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_test.vl"))
        })
        .collect();
    tests.sort();
    Ok(tests)
}

/// The `std` package directory, resolved in order (proposal/releases.md §3):
/// `$VILAN_STD`; the nearest ancestor of the entry file (then of the working
/// directory) containing `vilan/std/vilan.toml` — a checkout, so a `vilan`
/// built from this repo compiles against the working tree; else the binary's
/// own embedded std, materialized once to `~/.vilan/std-cache/<hash>/` — what
/// an installed binary uses, from any directory, with no checkout.
/// `resolve_std` reads the resulting package's `[library]` manifest (or, if
/// `$VILAN_STD` points at a bare source root with no manifest, uses it as the
/// base layer).
fn std_dir(entry: &Path) -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("VILAN_STD") {
        return Ok(PathBuf::from(path));
    }
    let starts = [
        entry
            .canonicalize()
            .ok()
            .and_then(|file| file.parent().map(Path::to_path_buf)),
        env::current_dir().ok(),
    ];
    for start in starts.iter().flatten() {
        let mut directory = Some(start.as_path());
        while let Some(current) = directory {
            let candidate = current.join("vilan").join("std");
            if candidate.join("vilan.toml").is_file() {
                return Ok(candidate);
            }
            directory = current.parent();
        }
    }
    vilan_embedded_std::materialize()
}

/// Runs the full pipeline (lex -> parse -> analyze -> contexts -> async infer ->
/// transform) over `file` and reports any diagnostics. Returns the JavaScript on
/// success, or a failure exit code (after reporting) on any error.
/// Writes the build's accumulated assets (const-eval.md §3) beside the
/// compiled output: `<output>.css` for kind "css", deduplicated and
/// deterministically ordered by `assemble_assets`.
fn write_assets(output_js: &std::path::Path, assets: &[(String, String)]) {
    for (kind, content) in vilan_core::const_eval::assemble_assets(assets) {
        let path = output_js.with_extension(kind.as_str());
        if let Err(error) = fs::write(&path, content) {
            eprintln!("error: cannot write {}: {error}", path.display());
        } else {
            println!("Emitted  {}", path.display());
        }
    }
}

fn compile_to_js(
    file: &Path,
    pkg_root: &Path,
    platform: Platform,
    options: &BuildOptions,
    workspace: &Workspace,
    emit_debug: bool,
    // When `Some`, an ANSI-free plain-text rendering of this file's diagnostics
    // is written here on a failed compile — the HMR error overlay's copy (hmr.md
    // §§2/§6). The terminal rendering below is untouched: this is a second,
    // additive pass over the SAME messages, never a redirect. Every other caller
    // passes `None` and pays nothing.
    overlay: Option<&mut String>,
) -> Result<(String, Vec<(String, String)>, Vec<(PathBuf, u64)>), ExitCode> {
    let src = match fs::read_to_string(file) {
        Ok(src) => src,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", file.display());
            return Err(ExitCode::FAILURE);
        }
    };
    let filename = file.to_string_lossy().into_owned();
    let std = match std_dir(file) {
        Ok(directory) => vilan_core::manifest::resolve_std(&directory),
        Err(error) => {
            eprintln!("error: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let mut output = None;

    // Fast path: a clean entry file reuses the shared content-addressed parse
    // cache (`vilan_core::parse_clean_cached`) — the same cache `std` and the
    // package modules use — so across `--watch` rounds an unchanged entry file
    // is served from the cache instead of re-parsed (backlog E12). A hit is
    // already lift-rewritten and `'static`; the handwritten frontend runs only
    // when the cache misses (a non-clean file), recovering a tree and naming its
    // diagnostics in a single fast-and-rich pass.
    let cached = vilan_core::parse_clean_cached(&src);

    // Analyzer and codegen diagnostics, collected as `(span, message)` for
    // ariadne; note-carrying ones render separately (they still count against a
    // clean build via `noted_errors`).
    let mut analyzer_errors: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut noted_errors = 0usize;
    // The same diagnostics, captured as structured items for the HMR overlay
    // (only assembled into text when `overlay` is `Some`). Populated alongside
    // the terminal path — never in place of it — reusing each message verbatim.
    let mut overlay_diagnostics: Vec<hmr::OverlayDiagnostic> = Vec::new();

    // On a cache miss the handwritten frontend parses the entry, always returning
    // a (possibly recovered) tree alongside every diagnostic. A batch compile does
    // not analyze a file that failed to parse cleanly — its parse errors are
    // reported and the build fails — so the freshly parsed tree is taken only when
    // the parse produced no diagnostics.
    let mut parse_errors: Vec<vilan_core::parsing::ParseError> = Vec::new();
    let fresh_root: Option<vilan_core::Spanned<vilan_core::node::NodeList>> = match &cached {
        None => {
            let (tree, errors) = vilan_core::parsing::parse(src.as_str());
            let clean = errors.is_empty();
            parse_errors = errors;
            tree.filter(|_| clean).map(|(mut items, span)| {
                // Bare-`?` marks become lift regions before analysis
                // (expression-lifting.md); the cached path is lifted inside
                // `parse_clean_cached`, so lift exactly once here.
                vilan_core::lift::rewrite_items(&mut items);
                (items, span)
            })
        }
        Some(_) => None,
    };
    let root: Option<&vilan_core::Spanned<vilan_core::node::NodeList>> = match &cached {
        Some((ast, _)) => Some(*ast),
        None => fresh_root.as_ref(),
    };
    // The source text the chosen root's spans index into: the cached `'static`
    // text on a hit (byte-identical to `src` — the cache is content-keyed),
    // otherwise `src` itself. Every diagnostic renders against it.
    let source_ref: &str = match &cached {
        Some((_, text)) => text,
        None => src.as_str(),
    };

    if let Some(root) = root {
        if emit_debug {
            write_debug(file, "parse.out", &format!("{root:#?}"));
        }

        let mut program = analyze(root, source_ref, &std, pkg_root, file, platform, workspace);

        // Thread `std::context::Context` values as hidden parameters (a no-op
        // unless the program creates a context).
        context::thread_contexts(&mut program);

        // Infer which functions/closures are async (drives `async`/`await`
        // code generation).
        async_infer::infer(&mut program);
        // Reject an async `drop` body now that asyncness is settled
        // (destruction.md §5): teardown must be synchronous in v1. An awaiting
        // body is async only by inference, so this runs after `async_infer`.
        vilan_core::analyzer::check_async_drops(&mut program);
        // Teardown must be context-free (destruction.md §8): a `drop` body whose
        // call sites (scope exits) can thread no context is rejected. Runs after
        // `thread_contexts` fills `context_dependent_functions`.
        vilan_core::analyzer::check_context_drops(&mut program);
        vilan_core::platform_color::check(&mut program, platform);

        // Evaluate `const` expressions (proposal/const-eval.md); the results
        // serialize in place at transform time, the failures are ordinary
        // diagnostics.
        let (const_results, const_assets, const_errors) = vilan_core::const_eval::evaluate(
            &program,
            &vilan_core::options::BuildOptions::default(),
        );
        program.const_results = const_results;
        program.const_assets = const_assets;
        program.diagnostics.extend(const_errors);

        for error in &program.diagnostics {
            // Capture every diagnostic for the overlay (message + note verbatim);
            // the terminal rendering below is unchanged.
            overlay_diagnostics.push(hmr::OverlayDiagnostic {
                span: error.span.into_range(),
                message: error.msg.clone(),
                note: error.note.as_ref().map(|note| note.msg.clone()),
            });
            // A note-carrying diagnostic renders directly (two labels — the
            // shared ariadne path has nowhere to put the secondary location);
            // plain ones keep the shared path.
            match &error.note {
                Some(note) => {
                    // A cross-source note reads its file so the sub-label can
                    // render in it (the trait's declaration in std, say).
                    let note_file = note
                        .source
                        .and_then(|source| {
                            let path = program.source_path(source)?;
                            let text = fs::read_to_string(path).ok()?;
                            Some((path.display().to_string(), text))
                        })
                        // The note's file may BE the entry — same-file
                        // rendering needs no second source.
                        .filter(|(name, _)| *name != filename);
                    report_error_with_note(&filename, source_ref, error, note_file);
                    noted_errors += 1;
                }
                None => analyzer_errors.push((error.span.into_range(), error.msg.clone())),
            }
        }
        // Warnings are non-fatal: render them, but they do not enter `errs`,
        // so they don't block codegen.
        for warning in &program.warnings {
            report_warning(
                &filename,
                source_ref,
                warning.span.into_range(),
                &warning.msg,
            );
        }

        if emit_debug {
            write_debug(file, "analyze.out", &format!("{program:#?}"));
            let call_graph = CallGraph::build(&program);
            write_debug(file, "callgraph.out", &call_graph.debug_dump(&program));
        }

        if analyzer_errors.is_empty() && noted_errors == 0 {
            match transform(&program, options) {
                // The leg's source set — each path paired with the content
                // hash it was COMPILED from — which the watch loop verifies
                // (by re-hashing, never by mtime) to skip a leg whose sources
                // didn't change (backlog E12, half b).
                Ok(javascript) => {
                    output = Some((
                        javascript,
                        program.const_assets.clone(),
                        program
                            .sources
                            .iter()
                            .cloned()
                            .zip(program.source_hashes.iter().copied())
                            .collect::<Vec<_>>(),
                    ))
                }
                Err(error) => {
                    overlay_diagnostics.push(hmr::OverlayDiagnostic {
                        span: error.span.into_range(),
                        message: error.msg.clone(),
                        note: error.note.as_ref().map(|note| note.msg.clone()),
                    });
                    analyzer_errors.push((error.span.into_range(), error.msg));
                }
            }
        }
    }

    let clean = analyzer_errors.is_empty() && parse_errors.is_empty() && noted_errors == 0;
    // The overlay's copy of this leg's diagnostics (hmr.md §§2/§6): the analyzer/
    // codegen items captured above, plus the parse errors rendered with the SAME
    // `render` the terminal `report` uses — only the location prefix and framing
    // are added here. Assembled only when a caller asked for it and the build
    // failed.
    if let Some(sink) = overlay {
        if !clean {
            for error in &parse_errors {
                overlay_diagnostics.push(hmr::OverlayDiagnostic {
                    span: error.span.into_range(),
                    message: vilan_core::parsing::render(error),
                    note: None,
                });
            }
            *sink = hmr::render_overlay(
                &filename,
                source_ref,
                &overlay_diagnostics,
                hmr::OVERLAY_DIAGNOSTIC_CAP,
            );
        }
    }
    report(&filename, source_ref, analyzer_errors, parse_errors);

    match output {
        Some(compiled) if clean => Ok(compiled),
        _ => Err(ExitCode::FAILURE),
    }
}

/// Writes `javascript` to a temp file and executes it with Node.js, propagating
/// its exit code, with stdin/stdout/stderr connected to the terminal. `args` are
/// forwarded to the program, reachable through `process::args()`. (A temp file
/// rather than piping via stdin, so the program keeps its own stdin — a piped
/// script would consume it, breaking `scan()`.)
fn run_node_script(javascript: &str, args: &[String]) -> ExitCode {
    let script = env::temp_dir().join(format!("vilan-run-{}.js", std::process::id()));
    if let Err(error) = fs::write(&script, javascript) {
        eprintln!("error: cannot write {}: {error}", script.display());
        return ExitCode::FAILURE;
    }
    let status = spawn_node(&script, args, None).and_then(|mut child| child.wait());
    let _ = fs::remove_file(&script);
    exit_code_of(status)
}

/// Spawns `node <script> <args...>` (optionally in `cwd`), inheriting this process's
/// stdio, and returns the child **without waiting**. `node <script>` makes the
/// program's `process.argv` `[node, script, ...args]`, so its `args()` (argv.slice(2))
/// sees exactly `args`. The caller either waits on it (`vilan run`) or holds the
/// handle to stop it on the next change (`vilan run --watch`).
fn spawn_node(script: &Path, args: &[String], cwd: Option<&Path>) -> std::io::Result<Child> {
    let mut command = std::process::Command::new("node");
    command.arg(script).args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.spawn()
}

/// Maps a launched process's result to an `ExitCode`, reporting a launch failure.
fn exit_code_of(status: std::io::Result<std::process::ExitStatus>) -> ExitCode {
    match status {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(code as u8),
            None => ExitCode::FAILURE, // terminated by a signal
        },
        Err(error) => {
            eprintln!(
                "error: failed to launch `node`: {error} \
                 (is Node.js installed and on your PATH?)"
            );
            ExitCode::FAILURE
        }
    }
}

/// Writes a `-d` debug dump alongside the source, warning (but not failing) on IO
/// error.
fn write_debug(file: &Path, extension: &str, contents: &str) {
    let path = file.with_extension(extension);
    if fs::write(&path, contents).is_err() {
        eprintln!("warning: failed to write {}", path.display());
    }
}

/// Renders parser diagnostics (via the handwritten frontend's `render`) and
/// analyzer/codegen diagnostics with ariadne. Analyzer diagnostics arrive
/// pre-rendered as `(span, message)`; parse errors carry the structured
/// found/expected/context/hint the renderer assembles.
fn report(
    filename: &str,
    src: &str,
    analyzer_errors: Vec<(std::ops::Range<usize>, String)>,
    parse_errors: Vec<vilan_core::parsing::ParseError>,
) {
    let diagnostics = analyzer_errors.into_iter().chain(
        parse_errors
            .into_iter()
            .map(|error| (error.span.into_range(), vilan_core::parsing::render(&error))),
    );
    for (span, message) in diagnostics {
        Report::build(ReportKind::Error, (filename.to_string(), span.clone()))
            .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
            .with_message(&message)
            .with_label(
                Label::new((filename.to_string(), span))
                    .with_message(&message)
                    .with_color(Color::Red),
            )
            .finish()
            .print(sources([(filename.to_string(), src.to_string())]))
            .unwrap()
    }
}

/// Renders one analyzer diagnostic that carries a secondary note
/// (diagnostics-standard.md C3): the primary label at the error's span, the
/// note as a second label at its own location ("first call here", "generated
/// by this attribute").
fn report_error_with_note(
    filename: &str,
    src: &str,
    error: &vilan_core::Error,
    // The note's own file when it lives elsewhere (name, contents) —
    // cross-source notes point into std or an imported module.
    note_file: Option<(String, String)>,
) {
    let Some(note) = &error.note else {
        return;
    };
    let note_filename = note_file
        .as_ref()
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| filename.to_string());
    let mut files = vec![(filename.to_string(), src.to_string())];
    if let Some((name, text)) = note_file {
        files.push((name, text));
    }
    Report::build(
        ReportKind::Error,
        (filename.to_string(), error.span.into_range()),
    )
    .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
    .with_message(error.msg.clone())
    .with_label(
        Label::new((filename.to_string(), error.span.into_range()))
            .with_message(error.msg.clone())
            .with_color(Color::Red),
    )
    .with_label(
        Label::new((note_filename, note.span.into_range()))
            .with_message(note.msg.clone())
            .with_color(Color::Yellow),
    )
    .finish()
    .print(sources(files))
    .unwrap();
}

/// Renders a single analyzer warning (e.g. an unused `[must_use]` result) — like
/// `report`, but `ReportKind::Warning` and non-fatal.
fn report_warning(filename: &str, src: &str, span: std::ops::Range<usize>, message: &str) {
    Report::build(ReportKind::Warning, (filename.to_string(), span.clone()))
        .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
        .with_message(message)
        .with_label(
            Label::new((filename.to_string(), span))
                .with_message(message)
                .with_color(Color::Yellow),
        )
        .finish()
        // stderr, so it doesn't corrupt `build --stdout` JS.
        .eprint(sources([(filename.to_string(), src.to_string())]))
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_roots_from_a_file_is_its_parent_directory() {
        // A non-existent `.vl` path isn't a directory, so it resolves to its parent —
        // where its `pkg::` siblings live.
        let roots = watch_roots(&Some(PathBuf::from("project/src/main.vl")));
        assert_eq!(roots, vec![PathBuf::from("project/src")]);
    }

    #[test]
    fn watch_roots_from_a_directory_is_the_directory() {
        // A real directory (so `is_dir()` holds) is watched as-is.
        let dir = env::temp_dir();
        assert_eq!(watch_roots(&Some(dir.clone())), vec![dir]);
    }

    #[test]
    fn scan_vl_tracks_only_vl_files_and_sees_additions() {
        let dir = env::temp_dir().join(format!("vilan-watch-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.vl"), "fun main() {}\n").unwrap();
        // A build's own output — must never be watched (else it triggers itself).
        fs::write(dir.join("a.js"), "// generated\n").unwrap();
        let roots = vec![dir.clone()];

        let snapshot = scan_vl(&roots);
        assert!(
            snapshot.keys().any(|path| path.ends_with("a.vl")),
            "the `.vl` source must be tracked"
        );
        assert!(
            !snapshot.keys().any(|path| path.ends_with("a.js")),
            "generated `.js` must not be tracked"
        );

        // Adding a `.vl` file changes the snapshot — a rebuild trigger.
        fs::write(dir.join("b.vl"), "fun helper() {}\n").unwrap();
        assert_ne!(scan_vl(&roots), snapshot);
        // Adding a `.js` file does not.
        let after_js = scan_vl(&roots);
        fs::write(dir.join("c.js"), "// also generated\n").unwrap();
        assert_eq!(scan_vl(&roots), after_js, "a new `.js` is not a change");

        let _ = fs::remove_dir_all(&dir);
    }
}
