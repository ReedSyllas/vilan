
use std::{collections::HashMap};
use chumsky::span::Span;

use crate::{analyzer::{Entity, Function, Id, Program}, shared::{BinaryOp, Error}};

pub fn transform<'src>(program: &Program<'src>) -> Result<String, Error> {
	Transformer::new(program, true).entry()
}

struct Transformer<'src> {
	program: &'src Program<'src>,
	ng: NameGenerator,
	required_functions: HashMap<Id, js::Node<'src>>,
	formatter: Formatter,
}

impl<'src> Transformer<'src> {
	fn new(program: &'src Program<'src>, should_pretty_print: bool) -> Self {
		let debug_names = if should_pretty_print {
			program.variables.iter().map(|(id, variable)| (*id, variable.name.to_string())).chain(program.functions.iter().map(|(id, function)| (*id, function.name.to_string()))).collect::<HashMap<Id, String>>()
		} else {
			HashMap::new()
		};
		
		Self {
			program,
			ng: NameGenerator::new("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ", debug_names),
			required_functions: HashMap::new(),
			formatter: if should_pretty_print { Formatter::new_pretty() } else { Formatter::new_compact() },
		}
	}
	
	fn entry(&mut self) -> Result<String, Error> {
		let main_fn_body = self.program.functions.values().find_map(|f| (f.name == "main").then_some(&f.body.0)).ok_or_else(|| Error {
			msg: "Cannot execute program without a main function".to_string(),
			span: Span::new((), 0..0),
		})?;
		
		let body = self.walk_list(main_fn_body);
		let functions = self.required_functions.values().collect::<Vec<_>>();
		
		Ok(format!("{}{}{}{}", self.formatter.file(&functions), self.formatter.line_break, self.formatter.file(&body.iter().map(|x| x).collect::<Vec<_>>()), self.formatter.line_break))
	}
	
	fn walk_list(&mut self, list: &Vec<Id>) -> Vec<js::Node<'src>> {
		list.iter().filter_map(|x| self.walk_entity(*x)).collect::<Vec<_>>()
	}
	
	fn walk_entity(&mut self, id: Id) -> Option<js::Node<'src>> {
		let entity = self.program.entity_map.get(&id).unwrap();
		
		Some(match entity {
			Entity::Error => unreachable!(),
			Entity::Void => js::Node::Void,
			Entity::Value(x) => js::Node::Value(x.clone()),
			Entity::Function(id) => {
				let function = self.program.functions.get(id).unwrap();
				self.function(function)
			},
			Entity::Local(id) => {
				js::Node::Local(self.ng.name_for(*id))
			},
			Entity::Call(subject_id, args) => {
				let subject = self.program.entity_map.get(subject_id).unwrap();
				match subject {
					Entity::Local(id) => {
						let args = args.iter().filter_map(|arg| self.walk_entity(*arg)).collect::<Vec<_>>();
						if *id == self.program.print_fn_id {
							js::Node::Call(
								Box::new(js::Node::Property(
									Box::new(js::Node::Local("console".to_string())),
									"log".to_string(),
								)),
								args,
							)
						} else {
							if !self.required_functions.contains_key(id) {
								if let Some(function) = self.program.functions.get(id) {
									let js_function = self.function(function);
									self.required_functions.insert(*id, js_function);
								}
							}
							let subject = self.ng.name_for(*id);
							js::Node::Call(Box::new(js::Node::Local(subject)), args)
						}
					}
					_ => unreachable!(),
				}
			},
			Entity::FunctionReturn(value) => js::Node::Return(Box::new(self.walk_entity(*value).unwrap_or(js::Node::Void))),
			Entity::Binary(op, lhs, rhs) => js::Node::Binary(*op, Box::new(self.walk_entity(*lhs).unwrap_or(js::Node::Void)), Box::new(self.walk_entity(*rhs).unwrap_or(js::Node::Void))),
			Entity::Variable(id) => {
				let name = self.ng.name_for(*id);
				let variable = self.program.variables.get(id).unwrap();
				let value = variable.initial.and_then(|id| self.walk_entity(id)).unwrap_or(js::Node::Void);
				js::Node::Variable(js::Variable {
					name,
					value: Box::new(value),
				})
			}
			Entity::Block(body) => {
				js::Node::Void
			}
			Entity::If(condition, then_body, else_arm) => {
				js::Node::Void
			}
			Entity::List(items) => {
				js::Node::Void
			}
		})
	}
	
	fn function(&mut self, function: &Function<'src>) -> js::Node<'src> {
		let name = self.ng.name_for(function.id);
		let parameters = function.parameters.iter().map(|parameter_id| js::Parameter { name: self.ng.name_for(*parameter_id) }).collect::<Vec<_>>();
		let body = self.walk_list(&function.body.0);
		js::Node::Function(js::Function { name, parameters, body })
	}
}

struct Formatter {
	line_break: &'static str,
	indentation: &'static str,
	space: &'static str,
}

impl Formatter {
	fn new_pretty() -> Self {
		Self {
			line_break: "\n",
			indentation: "\t",
			space: " ",
		}
	}
	
	fn new_compact() -> Self {
		Self {
			line_break: "",
			indentation: "",
			space: "",
		}
	}
	
	fn file(&self, list: &Vec<&js::Node>) -> String {
		self.sequence(list, ";", 0)
	}
	
	fn sequence(&self, list: &Vec<&js::Node>, terminator: &'static str, indentation: usize) -> String {
		list.iter().map(|node| self.node(node, terminator, indentation)).collect::<Vec<_>>().join(self.line_break)
	}
	
	fn node(&self, node: &js::Node, terminator: &'static str, indentation: usize) -> String {
		let text = match node {
			js::Node::Void => "undefined".to_string(),
			js::Node::Value(x) => format!("{}{}", x.to_string(), terminator),
			js::Node::Function(function) => {
				let name = function.name.as_str();
				let parameters = function.parameters.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(format!(",{}", self.space).as_str());
				let body = function.body.iter().map(|x| self.node(x, ";", indentation + 1)).collect::<Vec<_>>().join("");
				format!("function {}({}){}{{{}{}{}}}", name, parameters, self.space, self.line_break, body, self.line_break)
			}
			js::Node::Local(name) => format!("{}{}", name, terminator),
			js::Node::Return(value) => match &**value {
				js::Node::Void => format!("return{}", terminator),
				x => format!("return {}{}", self.node(x, "", 0), terminator),
			},
			js::Node::Call(subject, args) => {
				let s_subject = self.node(subject, "", 0);
				let s_args = args.iter().map(|x| self.node(x, "", 0)).collect::<Vec<_>>().join(format!(",{}", self.space).as_str());
				format!("{}({}){}", s_subject, s_args, terminator)
			}
			js::Node::Binary(op, lhs, rhs) => {
				let s_op = match op {
					BinaryOp::Add => "+",
					BinaryOp::Sub => "+",
					BinaryOp::Mul => "*",
					BinaryOp::Div => "/",
					BinaryOp::Eq => "===",
					BinaryOp::NotEq => "!==",
				};
				format!("{}{}{}{}{}{}", self.node(lhs, "", 0), self.space, s_op, self.space, self.node(rhs, "", 0), terminator)
			}
			js::Node::Variable(variable) => {
				let value = self.node(&variable.value, "", 0);
				format!("const {}{}={}{}{}", variable.name, self.space, self.space, value, terminator)
			}
			js::Node::Property(subject, member) => {
				let s_subject = self.node(subject, "", 0);
				format!("{}.{}{}", s_subject, member, terminator)
			}
		};
		
		format!("{}{}", self.indentation.repeat(indentation), text)
	}
}

pub mod js {
    use crate::shared::{BinaryOp, Value};

	#[derive(Clone, Debug)]
	pub enum Node<'src> {
		Binary(BinaryOp, Box<Self>, Box<Self>),
		Call(Box<Self>, Vec<Self>),
		Function(Function<'src>),
		Local(String),
		Property(Box<Self>, String),
		Return(Box<Self>),
		Value(Value<'src>),
		Variable(Variable<'src>),
		Void,
	}
	
	#[derive(Clone, Debug)]
	pub struct Function<'src> {
		pub name: String,
		pub parameters: Vec<Parameter>,
		pub body: Vec<Node<'src>>,
	}
	
	#[derive(Clone, Debug)]
	pub struct Parameter {
		pub name: String,
	}
	
	#[derive(Clone, Debug)]
	pub struct Variable<'src> {
		pub name: String,
		pub value: Box<Node<'src>>,
	}
}

struct NameGenerator {
	chars: Vec<char>,
	counter: u64,
	names: HashMap<Id, String>,
	debug_names: HashMap<Id, String>,
}

impl NameGenerator {
	pub fn new(chars: &str, debug_names: HashMap<Id, String>) -> Self {
		Self {
			chars: chars.chars().collect(),
			counter: 0,
			names: HashMap::new(),
			debug_names,
		}
	}
	
	pub fn name_for(&mut self, id: Id) -> String {
		self.names
		.get(&id)
		.map(|x| x.clone())
		.unwrap_or_else(|| {
			let debug_name = self.debug_names.get(&id).map(|x| x.clone());
			let name = debug_name.map(|x| format!("{}_{}", x, self.next_name())).unwrap_or_else(|| self.next_name());
			self.names.insert(id, name.clone());
			name
		})
	}
	
	fn next_idx(&mut self) -> u64 {
		let c = self.counter;
		self.counter += 1;
		c
	}
	
	fn next_name(&mut self) -> String {
		let c = self.next_idx();
		self.name_from_idx(c)
	}
	
	fn name_from_idx(&self, n: u64) -> String {
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
