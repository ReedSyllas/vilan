mod lexer;
mod parser;
mod shared;
mod analyzer;
mod transformer;

use ariadne::{sources, Color, Label, Report, ReportKind};
use chumsky::prelude::*;
use std::{env, fs, path::Path};

use crate::{analyzer::analyze, lexer::lexer, parser::parser, transformer::transform};

fn main() {
	let filename = env::args().nth(1).expect("Expected file argument");
	let src = fs::read_to_string(&filename).expect("Failed to read file");
	
	let (tokens, mut errs) = lexer().parse(src.as_str()).into_output_errors();
	
	let parse_errs = if let Some(tokens) = &tokens {
		let (ast, parse_errs) =
			parser()
			.map_with(|ast, e| (ast, e.span()))
			.parse(
				tokens
				.as_slice()
				.map((src.len()..src.len()).into(), |(t, s)| (t, s)),
			)
			.into_output_errors();
		
		if let Some((root, _file_span)) = ast.filter(|_| errs.len() + parse_errs.len() == 0) {
			println!("{root:?}");
			
			let program = analyze(&root);
			
			// println!("{program:#?}");
			
			match transform(&program) {
				Ok(output) => {
					println!("Output: {output}");
					fs::write(Path::new(&filename).with_extension("js"), output).unwrap_or_else(|_| {
						println!("failed to write file");
					});
				},
				Err(e) => errs.push(Rich::custom(e.span, e.msg)),
			}
			
			// match interpret(program) {
			// 	Ok(val) => println!("Return value: {val}"),
			// 	Err(e) => errs.push(Rich::custom(e.span, e.msg)),
			// }
			
			// match eval_expr(&root, &HashMap::new(), &mut Vec::new()) {
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
