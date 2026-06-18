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
        /// Also emit `.parse.out` / `.analyze.out` / `.callgraph.out` debug dumps.
        #[arg(short, long)]
        debug: bool,
    },
    /// Type-check, reporting diagnostics without writing output. With no path,
    /// checks the project entry from the nearest `vilan.toml`.
    Check {
        /// A `.vl` file, a project directory, or omitted to use `vilan.toml`.
        file: Option<PathBuf>,
        /// Also emit `.parse.out` / `.analyze.out` / `.callgraph.out` debug dumps.
        #[arg(short, long)]
        debug: bool,
    },
    /// Build and run a source file. (Not implemented yet.)
    Run {
        /// A `.vl` file, a project directory, or omitted to use `vilan.toml`.
        file: Option<PathBuf>,
    },
    /// Format vilan source files. (Not implemented yet.)
    Fmt {
        /// Files or directories to format.
        paths: Vec<PathBuf>,
    },
    /// Run a project's tests. (Not implemented yet.)
    Test {
        /// An optional name or path filter.
        filter: Option<String>,
    },
}

/// What `compile` does once a program type-checks: emit JavaScript (`build`) or
/// nothing (`check`). Both run the full pipeline so both surface every error.
enum Mode {
    Build { stdout: bool },
    Check,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Build {
            file,
            stdout,
            debug,
        } => with_entry(file, |entry| compile(entry, Mode::Build { stdout }, debug)),
        Command::Check { file, debug } => {
            with_entry(file, |entry| compile(entry, Mode::Check, debug))
        }
        // `run`/`fmt`/`test` await the project model, formatter, and test runner.
        Command::Run { .. } => unimplemented_command("run"),
        Command::Fmt { .. } => unimplemented_command("fmt"),
        Command::Test { .. } => unimplemented_command("test"),
    }
}

/// Resolves the entry source file from an optional path argument, then runs
/// `action` on it. A file path is compiled directly; a directory (or no path, via
/// the working directory) resolves the entry through its `vilan.toml`.
fn with_entry(path: Option<PathBuf>, action: impl FnOnce(&Path) -> ExitCode) -> ExitCode {
    match resolve_entry(path) {
        Ok(entry) => action(&entry),
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn resolve_entry(path: Option<PathBuf>) -> Result<PathBuf, String> {
    match path {
        // An explicit directory: compile the project rooted there.
        Some(path) if path.is_dir() => project_entry(&path),
        // An explicit file (or a not-yet-existing path, so `compile` can report
        // the read error): compile it directly.
        Some(path) => Ok(path),
        // No path: find the enclosing project from the working directory.
        None => {
            let working_dir = env::current_dir()
                .map_err(|error| format!("cannot read the working directory: {error}"))?;
            let root = find_project_root(&working_dir).ok_or_else(|| {
                "no `vilan.toml` found here or in any parent directory; \
                 pass a source file to compile it directly"
                    .to_string()
            })?;
            project_entry(&root)
        }
    }
}

/// The entry file a project's `vilan.toml` declares: `[package] entry` (relative
/// to the project root), defaulting to `main.vl`.
fn project_entry(root: &Path) -> Result<PathBuf, String> {
    let manifest = root.join("vilan.toml");
    let contents = fs::read_to_string(&manifest)
        .map_err(|error| format!("cannot read {}: {error}", manifest.display()))?;
    let table: toml::Table = toml::from_str(&contents)
        .map_err(|error| format!("invalid {}: {error}", manifest.display()))?;
    let entry = table
        .get("package")
        .and_then(|package| package.get("entry"))
        .and_then(|entry| entry.as_str())
        .unwrap_or("main.vl");
    Ok(root.join(entry))
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

/// A recognized subcommand whose implementation is still pending.
fn unimplemented_command(name: &str) -> ExitCode {
    eprintln!(
        "`vilan {name}` is not implemented yet — the toolchain currently supports \
         `build` and `check`."
    );
    ExitCode::FAILURE
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
/// transform) over `file`, reports any diagnostics, and — when clean — emits per
/// `mode`. Returns a failure exit code if anything went wrong.
fn compile(file: &Path, mode: Mode, emit_debug: bool) -> ExitCode {
    let src = match fs::read_to_string(file) {
        Ok(src) => src,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", file.display());
            return ExitCode::FAILURE;
        }
    };
    let filename = file.to_string_lossy().into_owned();
    let std_root = std_root();
    let mut emit_failed = false;

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

            let mut program = analyze(&root, &std_root, file);

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
                match transform(&program) {
                    Ok(output) => emit_failed = !emit(file, &mode, output),
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

    if clean && !emit_failed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Emits a successfully-transformed program per `mode`. Returns `false` if a file
/// write failed.
fn emit(file: &Path, mode: &Mode, output: String) -> bool {
    match mode {
        Mode::Build { stdout: true } => {
            print!("{output}");
            true
        }
        Mode::Build { stdout: false } => {
            let output_path = file.with_extension("js");
            match fs::write(&output_path, output) {
                Ok(()) => {
                    println!("Compiled {} -> {}", file.display(), output_path.display());
                    true
                }
                Err(error) => {
                    eprintln!("error: cannot write {}: {error}", output_path.display());
                    false
                }
            }
        }
        Mode::Check => {
            println!("{}: no errors", file.display());
            true
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
