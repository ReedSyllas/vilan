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
    /// Compile a source file to JavaScript, writing `<file>.js`.
    Build {
        /// The `.vl` source file to compile.
        file: PathBuf,
        /// Print the JavaScript to stdout instead of writing `<file>.js`.
        #[arg(long)]
        stdout: bool,
        /// Also emit `.parse.out` / `.analyze.out` / `.callgraph.out` debug dumps.
        #[arg(short, long)]
        debug: bool,
    },
    /// Type-check a source file, reporting diagnostics without writing output.
    Check {
        /// The `.vl` source file to check.
        file: PathBuf,
        /// Also emit `.parse.out` / `.analyze.out` / `.callgraph.out` debug dumps.
        #[arg(short, long)]
        debug: bool,
    },
    /// Build and run a source file. (Not implemented yet.)
    Run {
        /// The `.vl` source file to run.
        file: PathBuf,
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
        } => compile(&file, Mode::Build { stdout }, debug),
        Command::Check { file, debug } => compile(&file, Mode::Check, debug),
        // `run`/`fmt`/`test` await the project model, formatter, and test runner.
        Command::Run { .. } => unimplemented_command("run"),
        Command::Fmt { .. } => unimplemented_command("fmt"),
        Command::Test { .. } => unimplemented_command("test"),
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
