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
use vilan_core::parser::parser;
use vilan_core::transformer::transform;
use vilan_core::{BuildOptions, Preset, Target};

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
        /// The host to compile for: `node` (default) or `browser`.
        #[arg(long, default_value = "node")]
        target: String,
        /// Also emit `.parse.out` / `.analyze.out` / `.callgraph.out` debug dumps.
        #[arg(short, long)]
        debug: bool,
    },
    /// Type-check, reporting diagnostics without writing output. With no path,
    /// checks the project entry from the nearest `vilan.toml`.
    Check {
        /// A `.vl` file, a project directory, or omitted to use `vilan.toml`.
        file: Option<PathBuf>,
        /// The host to check for: `node` (default) or `browser`.
        #[arg(long, default_value = "node")]
        target: String,
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
            target,
            debug,
        } => match Target::parse(&target) {
            Some(target) => with_project(file, |project| match project {
                Project::Single { entry, options } => {
                    build(&entry, stdout, target, &options, debug)
                }
                Project::FullStack {
                    root,
                    server,
                    client,
                    options,
                } => build_fullstack(&root, &server, &client, &options, debug),
            }),
            None => unknown_target(&target),
        },
        Command::Check {
            file,
            target,
            debug,
        } => match Target::parse(&target) {
            Some(target) => with_project(file, |project| match project {
                Project::Single { entry, options } => check(&entry, target, &options, debug),
                Project::FullStack {
                    server,
                    client,
                    options,
                    ..
                } => check_fullstack(&server, &client, &options, debug),
            }),
            None => unknown_target(&target),
        },
        // `run`/`test` execute with `node`, so they are always Node builds.
        Command::Run { file, args } => with_project(file, |project| match project {
            Project::Single { entry, options } => run(&entry, &options, &args),
            Project::FullStack {
                root,
                server,
                client,
                options,
            } => run_fullstack(&root, &server, &client, &options, &args),
        }),
        Command::Test { path } => test(path),
        Command::Fmt { paths, check } => fmt(&paths, check),
    }
}

/// Reports an unrecognized `--target` value.
fn unknown_target(name: &str) -> ExitCode {
    eprintln!("error: unknown target `{name}` (expected `node` or `browser`)");
    ExitCode::FAILURE
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

/// A project to act on: a single entry, or a full-stack pair — a Node server plus
/// a browser client, declared by `[server]`/`[client]` in `vilan.toml`. Both
/// carry the code-generation options resolved from the manifest's `[build]`.
enum Project {
    Single {
        entry: PathBuf,
        options: BuildOptions,
    },
    FullStack {
        root: PathBuf,
        server: PathBuf,
        client: PathBuf,
        options: BuildOptions,
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
        // (there's no manifest to read `[build]` from).
        Some(path) => Ok(Project::Single {
            entry: path,
            options: BuildOptions::default(),
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

/// Reads a project's `vilan.toml`. `[server]` + `[client]` entries make a
/// full-stack project (a Node server + a browser client); otherwise `[package]
/// entry` (default `main.vl`) is a single entry. Entry paths are relative to the
/// project root.
fn project_from_manifest(root: &Path) -> Result<Project, String> {
    let manifest = root.join("vilan.toml");
    let contents = fs::read_to_string(&manifest)
        .map_err(|error| format!("cannot read {}: {error}", manifest.display()))?;
    let table: toml::Table = toml::from_str(&contents)
        .map_err(|error| format!("invalid {}: {error}", manifest.display()))?;
    let options = parse_build_options(&table)
        .map_err(|error| format!("invalid {}: {error}", manifest.display()))?;
    let entry = |section: &str| {
        table
            .get(section)
            .and_then(|section| section.get("entry"))
            .and_then(|entry| entry.as_str())
            .map(|entry| root.join(entry))
    };
    match (entry("server"), entry("client")) {
        (Some(server), Some(client)) => Ok(Project::FullStack {
            root: root.to_path_buf(),
            server,
            client,
            options,
        }),
        _ => {
            let single = table
                .get("package")
                .and_then(|package| package.get("entry"))
                .and_then(|entry| entry.as_str())
                .unwrap_or("main.vl");
            Ok(Project::Single {
                entry: root.join(single),
                options,
            })
        }
    }
}

/// Resolves the `[build]` code-generation options: a `preset` (default `debug`)
/// initializes every option, then individual keys override it.
fn parse_build_options(table: &toml::Table) -> Result<BuildOptions, String> {
    let Some(build) = table.get("build") else {
        return Ok(BuildOptions::default());
    };
    let mut options = match build.get("preset") {
        Some(value) => {
            let name = value.as_str().ok_or("`build.preset` must be a string")?;
            BuildOptions::from_preset(Preset::parse(name).ok_or_else(|| {
                format!("unknown build preset `{name}` (expected `debug` or `release`)")
            })?)
        }
        None => BuildOptions::default(),
    };
    let flag = |key: &str, current: bool| -> Result<bool, String> {
        match build.get(key) {
            Some(value) => value
                .as_bool()
                .ok_or_else(|| format!("`build.{key}` must be a boolean")),
            None => Ok(current),
        }
    };
    options.indent = flag("indent", options.indent)?;
    options.spaces = flag("spaces", options.spaces)?;
    options.readable_names = flag("readable-names", options.readable_names)?;
    options.debug_names = flag("debug-names", options.debug_names)?;
    Ok(options)
}

/// Builds a full-stack project into `<root>/dist/`: the client for the browser
/// (`dist/client.js`) and the server for Node (`dist/server.js`). The client is
/// built first since the server serves `dist/client.js`. `--target`/`--stdout`
/// don't apply — each entry has its own target and output.
fn build_fullstack(
    root: &Path,
    server: &Path,
    client: &Path,
    options: &BuildOptions,
    debug: bool,
) -> ExitCode {
    match build_fullstack_artifacts(root, server, client, options, debug) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn build_fullstack_artifacts(
    root: &Path,
    server: &Path,
    client: &Path,
    options: &BuildOptions,
    debug: bool,
) -> Result<(), ExitCode> {
    let dist = root.join("dist");
    if let Err(error) = fs::create_dir_all(&dist) {
        eprintln!("error: cannot create {}: {error}", dist.display());
        return Err(ExitCode::FAILURE);
    }
    for (entry, target, output_name) in [
        (client, Target::Browser, "client.js"),
        (server, Target::Node, "server.js"),
    ] {
        let javascript = compile_to_js(entry, target, options, debug)?;
        let output = dist.join(output_name);
        if let Err(error) = fs::write(&output, javascript) {
            eprintln!("error: cannot write {}: {error}", output.display());
            return Err(ExitCode::FAILURE);
        }
        println!("Compiled {} -> {}", entry.display(), output.display());
    }
    Ok(())
}

/// Type-checks both entries of a full-stack project (the client for the browser,
/// the server for Node).
fn check_fullstack(server: &Path, client: &Path, options: &BuildOptions, debug: bool) -> ExitCode {
    let client_ok = compile_to_js(client, Target::Browser, options, debug).is_ok();
    let server_ok = compile_to_js(server, Target::Node, options, debug).is_ok();
    if client_ok && server_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Builds a full-stack project, then runs its server with `node` from the project
/// root (so it can read `dist/client.js`). `args` are forwarded to the server.
fn run_fullstack(
    root: &Path,
    server: &Path,
    client: &Path,
    options: &BuildOptions,
    args: &[String],
) -> ExitCode {
    if let Err(code) = build_fullstack_artifacts(root, server, client, options, false) {
        return code;
    }
    // Run from the project root so the server reads `dist/client.js`; the script
    // path is relative to that working directory.
    let status = std::process::Command::new("node")
        .arg(Path::new("dist").join("server.js"))
        .args(args)
        .current_dir(root)
        .status();
    match status {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(code as u8),
            None => ExitCode::FAILURE,
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
    let javascript = compile_to_js(file, Target::Node, &BuildOptions::default(), false)
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

/// The `std` package source root: `$VILAN_STD` if set, else the in-repo
/// `vilan/std/src` relative to this crate.
fn std_root() -> PathBuf {
    env::var_os("VILAN_STD")
        .map(PathBuf::from)
        // `CARGO_MANIFEST_DIR` is `crates/vilan-cli`; std lives at the workspace
        // root under `vilan/std/src`.
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std/src"))
}

/// Runs the full pipeline (lex -> parse -> analyze -> contexts -> async infer ->
/// transform) over `file` and reports any diagnostics. Returns the JavaScript on
/// success, or a failure exit code (after reporting) on any error.
fn compile_to_js(
    file: &Path,
    target: Target,
    options: &BuildOptions,
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
    let std_root = std_root();
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

            let mut program = analyze(&root, &std_root, file, target);

            // Thread `std::context::Context` values as hidden parameters (a no-op
            // unless the program creates a context).
            context::thread_contexts(&mut program);

            // Infer which functions/closures are async (drives `async`/`await`
            // code generation).
            async_infer::infer(&mut program);

            for error in &program.diagnostics {
                errs.push(Rich::custom(error.span, error.msg.as_str()));
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

/// Compiles `file` and writes `<file>.js` (or prints to stdout).
fn build(
    file: &Path,
    stdout: bool,
    target: Target,
    options: &BuildOptions,
    emit_debug: bool,
) -> ExitCode {
    let javascript = match compile_to_js(file, target, options, emit_debug) {
        Ok(javascript) => javascript,
        Err(code) => return code,
    };
    if stdout {
        print!("{javascript}");
        return ExitCode::SUCCESS;
    }
    let output_path = file.with_extension("js");
    match fs::write(&output_path, javascript) {
        Ok(()) => {
            println!("Compiled {} -> {}", file.display(), output_path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: cannot write {}: {error}", output_path.display());
            ExitCode::FAILURE
        }
    }
}

/// Compiles `file` and reports diagnostics, writing no output.
fn check(file: &Path, target: Target, options: &BuildOptions, emit_debug: bool) -> ExitCode {
    match compile_to_js(file, target, options, emit_debug) {
        Ok(_) => {
            println!("{}: no errors", file.display());
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// Compiles `file`, then executes the JavaScript with Node.js — propagating its
/// exit code, with stdin/stdout/stderr connected to the terminal. `args` are
/// forwarded to the program, reachable through `process::args()`.
fn run(file: &Path, options: &BuildOptions, args: &[String]) -> ExitCode {
    let javascript = match compile_to_js(file, Target::Node, options, false) {
        Ok(javascript) => javascript,
        Err(code) => return code,
    };
    // Run from a temp file rather than piping the script via stdin, so the program
    // keeps its own stdin (a piped script would consume it, breaking `scan()`).
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
