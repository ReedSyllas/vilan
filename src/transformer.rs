use std::{collections::{HashMap, HashSet}, slice::SliceIndex};

use chumsky::{input::Input, span::Span};

use crate::{analyzer::{Entity, EntityId, Function, Program, Scope}, shared::Error};

pub fn transform<'src>(program: &Program<'src>) -> Result<String, Error> {
	let mut transformer = Transformer::new(program, true);
	transformer.entry()
}

struct Transformer<'src> {
	program: &'src Program<'src>,
	ng: NameGenerator,
	global_header: String,
	required_functions: HashSet<EntityId>,
	fmt_line_break: &'static str,
	fmt_indentation: &'static str,
	fmt_space: &'static str,
}

impl<'src> Transformer<'src> {
	fn new(program: &'src Program<'src>, should_pretty_print: bool) -> Self {
		Self {
			program,
			ng: NameGenerator::new("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"),
			global_header: String::new(),
			required_functions: HashSet::new(),
			fmt_line_break: if should_pretty_print { "\n" } else { "" },
			fmt_indentation: if should_pretty_print { "\t" } else { "" },
			fmt_space: if should_pretty_print { " " } else { "" },
		}
	}
	
	fn ind(&self, depth: usize) -> String {
		self.fmt_indentation.repeat(depth)
	}
	
	fn entry(&mut self) -> Result<String, Error> {
		let entry = self.program.get_main_entry().ok_or_else(|| Error {
			msg: "Cannot execute program without a main function".to_string(),
			span: Span::new((), 0..0),
		})?;
		let body = self.entity(entry, ";", 0).unwrap_or_else(|| String::new());
		let head = self.global_header.as_str();
		Ok(self.fmt_line_break.to_string() + head + self.fmt_line_break.repeat(2).as_str() + body.as_str() + self.fmt_line_break)
	}
	
	fn entity(&mut self, entity: &Entity<'src>, terminator: &'static str, depth: usize) -> Option<String> {
		Some(match entity {
			Entity::Error => unreachable!(),
			Entity::Value(val) => val.clone().to_string() + terminator,
			Entity::Function(id) => {
				let function = self.program.get_function(id).unwrap();
				self.function(function, terminator, depth)
			}
			Entity::Local(id) => {
				let variable = self.program.get_variable(id).unwrap();
				self.ind(depth) + format!("{}", self.ng.name_for(*id)).as_str() + terminator
			}
			Entity::Call(subject, args) => {
				match &**subject {
					Entity::Local(id) => {
						if !self.required_functions.contains(id) {
							self.required_functions.insert(*id);
							let function = self.program.get_function(id).unwrap();
							let function_string = self.function(function, terminator, 0);
							self.global_header += function_string.as_str();
						}
						let args_string = args.iter().filter_map(|arg| self.entity(arg, "", depth)).collect::<Vec<_>>().join(",");
						self.ind(depth) + format!("{}({})", self.ng.name_for(*id), args_string).as_str() + terminator
					}
					_ => unreachable!(),
				}
			}
			Entity::Seq(children) => {
				children.iter().filter_map(|child| self.entity(child, terminator, depth)).collect::<Vec<_>>().join(self.fmt_line_break)
			}
		})
	}
	
	fn function(&mut self, function: &Function<'src>, terminator: &'static str, depth: usize) -> String {
		let name = self.ng.name_for(function.id);
		let parameters_string = function.parameters.iter().map(|parameter| self.ng.name_for(parameter.id)).collect::<Vec<_>>().join(",");
		let body_string = self.entity(&function.body, ";", depth + 1).unwrap_or_else(|| String::new());
		self.ind(depth) + format!("function {}({}){}{{{}{}{}}}", name, parameters_string, self.fmt_space, self.fmt_line_break, body_string, self.fmt_line_break).as_str() + match terminator {
			";" => "",
			x => x,
		}
	}
}

struct NameGenerator {
	chars: Vec<char>,
	counter: u64,
	names: HashMap<EntityId, String>,
}

impl NameGenerator {
	pub fn new(chars: &str) -> Self {
		Self {
			chars: chars.chars().collect(),
			counter: 0,
			names: HashMap::new(),
		}
	}
	
	pub fn name_for(&mut self, id: EntityId) -> String {
		self.names
		.get(&id)
		.map(|x| x.clone())
		.unwrap_or_else(|| {
			let name = self.next();
			self.names.insert(id, name.clone());
			name
		})
	}
	
	pub fn next(&mut self) -> String {
		let s = self.encode(self.counter);
		self.counter += 1;
		s
	}
	
	fn encode(&self, n: u64) -> String {
		let mut s = String::new();
		let mut num = n;
		let base = self.chars.len() as u64;
		
		loop {
			let remainder = (num % base) as usize;
			s.push(self.chars[remainder]);
			num /= base;
			if num < 1 {
				break;
			}
			num -= 1;
		}
		
		s.chars().rev().collect()
	}
}
