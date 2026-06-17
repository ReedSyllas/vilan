use vilan_core::analyzer::analyze;
use vilan_core::async_infer;
use vilan_core::call_graph::CallGraph;
use vilan_core::context;
use vilan_core::lexer::lexer;
use vilan_core::parser::parser;
use vilan_core::transformer::transform;

use ariadne::{Color, Label, Report, ReportKind, sources};
use chumsky::prelude::*;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    // Flags may appear in any position; the file is the first non-flag arg.
    //   -d  also emit the `FILE.parse.out` / `FILE.analyze.out` debug files
    //   -r  print the JS to stdout instead of writing `FILE.js`
    let args: Vec<String> = env::args().skip(1).collect();
    let emit_debug = args.iter().any(|arg| arg == "-d");
    let print_output = args.iter().any(|arg| arg == "-r");
    let filename = args
        .iter()
        .find(|arg| !arg.starts_with('-'))
        .cloned()
        .expect("Expected file argument");
    let src = fs::read_to_string(&filename).expect("Failed to read file");

    // The `std` package's source root: `$VILAN_STD` if set, else the in-repo
    // `vilan/std/src` relative to the crate.
    let std_root: PathBuf = env::var_os("VILAN_STD")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // `CARGO_MANIFEST_DIR` is `crates/vilan-cli`; the std sources live at the
            // workspace root under `vilan/std/src`.
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vilan/std/src")
        });

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
                fs::write(
                    Path::new(&filename).with_extension("parse.out"),
                    format!("{root:#?}"),
                )
                .unwrap_or_else(|_| {
                    println!("failed to write parse.out");
                });
            }

            let mut program = analyze(&root, &std_root, Path::new(&filename));

            // Thread `std::context::Context` values as hidden parameters (a
            // no-op unless the program creates a context).
            context::thread_contexts(&mut program);

            // Infer which functions/closures are async (drives `async`/`await`
            // code generation).
            async_infer::infer(&mut program);

            for error in &program.diagnostics {
                errs.push(Rich::custom(error.span, error.msg.as_str()));
            }

            if emit_debug {
                fs::write(
                    Path::new(&filename).with_extension("analyze.out"),
                    format!("{program:#?}"),
                )
                .unwrap_or_else(|_| {
                    println!("failed to write analyze.out");
                });

                let call_graph = CallGraph::build(&program);
                fs::write(
                    Path::new(&filename).with_extension("callgraph.out"),
                    call_graph.debug_dump(&program),
                )
                .unwrap_or_else(|_| {
                    println!("failed to write callgraph.out");
                });
            }

            if errs.len() == 0 {
                match transform(&program) {
                    Ok(output) => {
                        if print_output {
                            print!("{output}");
                        } else {
                            fs::write(Path::new(&filename).with_extension("js"), output)
                                .unwrap_or_else(|_| {
                                    println!("failed to write file");
                                });
                            println!("Package has built successfully");
                        }
                    }
                    Err(e) => errs.push(Rich::custom(e.span, e.msg)),
                }
            }

            // match interpret(program) {
            // 	Ok(val) => println!("Return value: {val}"),
            // 	Err(e) => errs.push(Rich::custom(e.span, e.msg)),
            // }

            // match eval_expr(&root, &IndexMap::new(), &mut Vec::new()) {
            // 	Ok(val) => println!("Return value: {val}"),
            // 	Err(e) => errs.push(Rich::custom(e.span, e.msg)),
            // }
        }

        // if let Some((functions, file_span)) = ast.filter(|_| errs.len() + parse_errs.len() == 0) {
        // 	if let Some(main) = functions.get("main") {
        // 		if !main.args.is_empty() {
        // 			errs.push(Rich::custom(
        // 				main.span,
        // 				"The main function cannot have arguments".to_string(),
        // 			))
        // 		} else {
        // 			let body = &main.body;
        // 			println!("{body:#?}");

        // 			// match eval_expr(body, &functions, &mut Vec::new()) {
        // 			// 	Ok(val) => println!("Return value: {val}"),
        // 			// 	Err(e) => errs.push(Rich::custom(e.span, e.msg)),
        // 			// }
        // 		}
        // 	} else {
        // 		errs.push(Rich::custom(
        // 			file_span,
        // 			"Programs need a main function but none was found".to_string(),
        // 		));
        // 	}
        // }

        parse_errs
    } else {
        Vec::new()
    };

    errs.into_iter()
        .map(|e| e.map_token(|c| c.to_string()))
        .chain(
            parse_errs
                .into_iter()
                .map(|e| e.map_token(|tok| tok.to_string())),
        )
        .for_each(|e| {
            Report::build(ReportKind::Error, (filename.clone(), e.span().into_range()))
                .with_config(ariadne::Config::new().with_index_type(ariadne::IndexType::Byte))
                .with_message(e.to_string())
                .with_label(
                    Label::new((filename.clone(), e.span().into_range()))
                        .with_message(e.reason().to_string())
                        .with_color(Color::Red),
                )
                .with_labels(e.contexts().map(|(label, span)| {
                    Label::new((filename.clone(), span.into_range()))
                        .with_message(format!("while parsing this {label}"))
                        .with_color(Color::Yellow)
                }))
                .finish()
                .print(sources([(filename.clone(), src.clone())]))
                .unwrap()
        });
}
