use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::prelude::*;
// `clap::Parser` collides with `chumsky`'s `Parser` trait (glob-imported above),
// so bring it in anonymously — enough for `Cli::parse()` — and derive by path.
use clap::{Parser as _, Subcommand};
use vilan_core::analyzer::analyze;
use vilan_core::async_infer;
use vilan_core::call_graph::CallGraph;
use vilan_core::context;
use vilan_core::lexer::lexer;
use vilan_core::manifest::{EntrySection, Package};
use vilan_core::parser::parser;
use vilan_core::transformer::transform;
use vilan_core::{Backend, BuildOptions, Manifest, Platform, Workspace};

/// The vilan language toolchain.
#[derive(clap::Parser)]
#[command(name = "vilan", version, about)]
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
        /// The platform to build for: `node` (`node:24`), `deno` (`deno:2`),
        /// `browser`, or `none`. Overrides the package's `target`; defaults to it,
        /// else `node`. `--target` is an accepted alias.
        #[arg(long, alias = "target")]
        platform: Option<String>,
        /// The emitter backend: `js` (the only backend today).
        #[arg(long)]
        backend: Option<String>,
        /// Also emit `.parse.out` / `.analyze.out` / `.callgraph.out` debug dumps.
        #[arg(short, long)]
        debug: bool,
    },
    /// Type-check, reporting diagnostics without writing output. With no path,
    /// checks the project entry from the nearest `vilan.toml`.
    Check {
        /// A `.vl` file, a project directory, or omitted to use `vilan.toml`.
        file: Option<PathBuf>,
        /// The platform to check for: `node` (`node:24`), `deno` (`deno:2`),
        /// `browser`, or `none`. Overrides the package's `target`; defaults to it,
        /// else `node`. `--target` is an accepted alias.
        #[arg(long, alias = "target")]
        platform: Option<String>,
        /// The emitter backend: `js` (the only backend today).
        #[arg(long)]
        backend: Option<String>,
        /// Also emit `.parse.out` / `.analyze.out` / `.callgraph.out` debug dumps.
        #[arg(short, long)]
        debug: bool,
    },
    /// Build and run a source file, forwarding any trailing arguments to the
    /// program (reach them with `process::args()`).
    Run {
        /// A `.vl` file, a project directory, or omitted to use `vilan.toml`.
        file: Option<PathBuf>,
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
        } => match effective_backend(backend.as_deref()) {
            Err(message) => report_error(&message),
            Ok(_backend) => with_project(file, |project| match project {
                Project::Single {
                    unit,
                    platform: package_platform,
                } => match effective_platform(platform.as_deref(), package_platform) {
                    Ok(Platform::None) => no_host_platform(),
                    Ok(platform) => build_single(&unit, stdout, platform, debug),
                    Err(message) => report_error(&message),
                },
                // A workspace builds each member for its own declared platform, so
                // the `--platform` flag doesn't apply.
                Project::Workspace { root, members } => build_workspace(&root, &members, debug),
            }),
        },
        Command::Check {
            file,
            platform,
            backend,
            debug,
        } => match effective_backend(backend.as_deref()) {
            Err(message) => report_error(&message),
            Ok(_backend) => with_project(file, |project| match project {
                Project::Single {
                    unit,
                    platform: package_platform,
                } => match effective_platform(platform.as_deref(), package_platform) {
                    // A `none` package is a pure library — it can't be built, but it
                    // can still be type-checked (against the base layer only).
                    Ok(platform) => check_single(&unit, platform, debug),
                    Err(message) => report_error(&message),
                },
                Project::Workspace { members, .. } => check_workspace(&members, debug),
            }),
        },
        // `run`/`test` execute with `node`.
        Command::Run { file, args } => with_project(file, |project| match project {
            Project::Single { unit, platform } => {
                let platform = platform.unwrap_or_default();
                if matches!(platform, Platform::Node { .. }) {
                    run_single(&unit, &args)
                } else {
                    eprintln!(
                        "error: `vilan run` executes with Node, but the package platform is `{}`",
                        platform.name()
                    );
                    ExitCode::FAILURE
                }
            }
            Project::Workspace { root, members } => run_workspace(&root, &members, &args),
        }),
        Command::Test { path } => test(path),
        Command::Fmt { paths, check } => fmt(&paths, check),
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

/// Resolves the project rooted at `directory` from its `vilan.toml`. A `[package]`
/// is a single package (`entry` resolves against `root`; `target` is the default).
/// A `[project]` is a workspace — each member builds for its own platform. The legacy
/// `[server]`/`[client]` pair lowers onto a two-member workspace.
fn project_from_manifest(directory: &Path) -> Result<Project, String> {
    let manifest = read_manifest(directory)?;
    let options = manifest
        .build_options()
        .map_err(|error| format!("invalid {}/vilan.toml: {error}", directory.display()))?;

    // Legacy full-stack: a browser client + a Node server, as a two-member
    // workspace (client first, since the server serves `dist/client.js`).
    if manifest.is_legacy_fullstack() {
        let member = |section: &Option<EntrySection>, name: &str, platform: Platform| {
            let entry = directory.join(
                section
                    .as_ref()
                    .and_then(|section| section.entry.as_deref())
                    .unwrap_or(Path::new("main.vl")),
            );
            (
                Unit {
                    name: name.to_string(),
                    pkg_root: pkg_root_of(&entry),
                    entry,
                    package_dir: None,
                    options,
                },
                platform,
            )
        };
        return Ok(Project::Workspace {
            root: directory.to_path_buf(),
            members: vec![
                member(&manifest.client, "client", Platform::Browser),
                member(&manifest.server, "server", Platform::default()),
            ],
        });
    }

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
            let platform = package.resolved_target().unwrap_or_default();
            members.push((
                unit_from_package(&member_dir, package, member_options),
                platform,
            ));
        }
        return Ok(Project::Workspace {
            root: directory.to_path_buf(),
            members,
        });
    }

    // A single package. `validate` guarantees `[package]` is present here.
    let package = manifest.package.as_ref().expect("validated package");
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
fn compile_unit(unit: &Unit, platform: Platform, emit_debug: bool) -> Result<String, ExitCode> {
    let workspace = match resolve_workspace(unit) {
        Ok(workspace) => workspace,
        Err(message) => {
            eprintln!("error: {message}");
            return Err(ExitCode::FAILURE);
        }
    };
    compile_to_js(
        &unit.entry,
        &unit.pkg_root,
        platform,
        &unit.options,
        &workspace,
        emit_debug,
    )
}

/// Builds a lone package / bare file, writing `<entry>.js` (or printing to stdout).
fn build_single(unit: &Unit, stdout: bool, platform: Platform, emit_debug: bool) -> ExitCode {
    let javascript = match compile_unit(unit, platform, emit_debug) {
        Ok(javascript) => javascript,
        Err(code) => return code,
    };
    if stdout {
        print!("{javascript}");
        return ExitCode::SUCCESS;
    }
    let output_path = unit.entry.with_extension("js");
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
    match compile_unit(unit, platform, emit_debug) {
        Ok(_) => {
            println!("{}: no errors", unit.entry.display());
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// Builds and runs a lone package's entry with Node, forwarding `args`.
fn run_single(unit: &Unit, args: &[String]) -> ExitCode {
    let javascript = match compile_unit(unit, Platform::default(), false) {
        Ok(javascript) => javascript,
        Err(code) => return code,
    };
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
        let javascript = compile_unit(unit, *platform, debug)?;
        let output = dist.join(format!("{}.js", unit.name));
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
        ok &= compile_unit(unit, *platform, debug).is_ok();
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Builds a workspace, then runs its single Node member with `node` from the
/// project root (so it can read sibling `dist/*.js`). `args` are forwarded.
fn run_workspace(root: &Path, members: &[(Unit, Platform)], args: &[String]) -> ExitCode {
    let node_members: Vec<&Unit> = members
        .iter()
        .filter(|(_, platform)| matches!(platform, Platform::Node { .. }))
        .map(|(unit, _)| unit)
        .collect();
    let server = match node_members.as_slice() {
        [unit] => unit,
        [] => {
            eprintln!("error: no `node` package in this workspace to run");
            return ExitCode::FAILURE;
        }
        _ => {
            eprintln!(
                "error: this workspace has more than one `node` package; \
                 `vilan run` can't tell which to run"
            );
            return ExitCode::FAILURE;
        }
    };
    if let Err(code) = build_workspace_artifacts(root, members, false) {
        return code;
    }
    // Run from the project root so the server reads sibling `dist/*.js`; the script
    // path is relative to that working directory.
    let status = std::process::Command::new("node")
        .arg(Path::new("dist").join(format!("{}.js", server.name)))
        .args(args)
        .current_dir(root)
        .status();
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
    let javascript = compile_to_js(
        file,
        &pkg_root_of(file),
        Platform::default(),
        &BuildOptions::default(),
        &Workspace::default(),
        false,
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

/// The `std` library directory: `$VILAN_STD` if set, else the in-repo `vilan/std`
/// relative to this crate. `resolve_std` reads its `[library]` manifest (or, if
/// `$VILAN_STD` points at a bare source root with no manifest, uses it as the base
/// layer).
fn std_dir() -> PathBuf {
    env::var_os("VILAN_STD")
        .map(PathBuf::from)
        // `CARGO_MANIFEST_DIR` is `crates/vilan-cli`; std lives at the workspace
        // root under `vilan/std`.
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std"))
}

/// Runs the full pipeline (lex -> parse -> analyze -> contexts -> async infer ->
/// transform) over `file` and reports any diagnostics. Returns the JavaScript on
/// success, or a failure exit code (after reporting) on any error.
fn compile_to_js(
    file: &Path,
    pkg_root: &Path,
    platform: Platform,
    options: &BuildOptions,
    workspace: &Workspace,
    emit_debug: bool,
) -> Result<String, ExitCode> {
    let src = match fs::read_to_string(file) {
        Ok(src) => src,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", file.display());
            return Err(ExitCode::FAILURE);
        }
    };
    let filename = file.to_string_lossy().into_owned();
    let std = vilan_core::manifest::resolve_std(&std_dir());
    let mut output = None;

    let (tokens, mut errs) = lexer().parse(src.as_str()).into_output_errors();

    let parse_errs = if let Some(tokens) = &tokens {
        let (ast, parse_errs) = parser()
            .map_with(|ast, e| (ast, e.span()))
            .parse(
                tokens
                    .as_slice()
                    .map((src.len()..src.len()).into(), |(t, s)| (t, s)),
            )
            .into_output_errors();

        if let Some((root, _file_span)) = ast.filter(|_| errs.len() + parse_errs.len() == 0) {
            if emit_debug {
                write_debug(file, "parse.out", &format!("{root:#?}"));
            }

            let mut program = analyze(&root, &std, pkg_root, file, platform, workspace);

            // Thread `std::context::Context` values as hidden parameters (a no-op
            // unless the program creates a context).
            context::thread_contexts(&mut program);

            // Infer which functions/closures are async (drives `async`/`await`
            // code generation).
            async_infer::infer(&mut program);

            for error in &program.diagnostics {
                errs.push(Rich::custom(error.span, error.msg.as_str()));
            }
            // Warnings are non-fatal: render them, but they do not enter `errs`,
            // so they don't block codegen.
            for warning in &program.warnings {
                report_warning(&filename, &src, warning.span.into_range(), &warning.msg);
            }

            if emit_debug {
                write_debug(file, "analyze.out", &format!("{program:#?}"));
                let call_graph = CallGraph::build(&program);
                write_debug(file, "callgraph.out", &call_graph.debug_dump(&program));
            }

            if errs.is_empty() {
                match transform(&program, options) {
                    Ok(javascript) => output = Some(javascript),
                    Err(error) => errs.push(Rich::custom(error.span, error.msg)),
                }
            }
        }

        parse_errs
    } else {
        Vec::new()
    };

    let clean = errs.is_empty() && parse_errs.is_empty();
    report(&filename, &src, errs, parse_errs);

    match output {
        Some(javascript) if clean => Ok(javascript),
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
    // `node <script> <args...>` — `process.argv` becomes `[node, script, ...args]`,
    // so the program's `args()` (argv.slice(2)) sees exactly `args`.
    let status = std::process::Command::new("node")
        .arg(&script)
        .args(args)
        .status();
    let _ = fs::remove_file(&script);
    exit_code_of(status)
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

/// Renders lexer + parser diagnostics with ariadne (analyzer diagnostics arrive
/// folded into `errs` as `Rich::custom`).
fn report<'src>(
    filename: &str,
    src: &'src str,
    errs: Vec<Rich<'src, char>>,
    parse_errs: Vec<Rich<'src, vilan_core::token::Token<'src>>>,
) {
    errs.into_iter()
        .map(|error| error.map_token(|character| character.to_string()))
        .chain(
            parse_errs
                .into_iter()
                .map(|error| error.map_token(|token| token.to_string())),
        )
        .for_each(|error| {
            Report::build(
                ReportKind::Error,
                (filename.to_string(), error.span().into_range()),
            )
            .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
            .with_message(error.to_string())
            .with_label(
                Label::new((filename.to_string(), error.span().into_range()))
                    .with_message(error.reason().to_string())
                    .with_color(Color::Red),
            )
            .with_labels(error.contexts().map(|(label, span)| {
                Label::new((filename.to_string(), span.into_range()))
                    .with_message(format!("while parsing this {label}"))
                    .with_color(Color::Yellow)
            }))
            .finish()
            .print(sources([(filename.to_string(), src.to_string())]))
            .unwrap()
        });
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
