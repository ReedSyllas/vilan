
use std::collections::{HashMap};
use chumsky::span::Span;

use crate::{analyzer::{Entity, EntityId, Function, Program}, shared::Error};

pub fn transform<'src>(program: &Program<'src>) -> Result<String, Error> {
	Transformer::new(program, true).entry()
}

struct Transformer<'src> {
	program: &'src Program<'src>,
	ng: NameGenerator,
	required_functions: HashMap<EntityId, js::Node<'src>>,
	fmt_line_break: &'static str,
	fmt_indentation: &'static str,
	fmt_space: &'static str,
}

impl<'src> Transformer<'src> {
	fn new(program: &'src Program<'src>, should_pretty_print: bool) -> Self {
		Self {
			program,
			ng: NameGenerator::new("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"),
			required_functions: HashMap::new(),
			fmt_line_break: if should_pretty_print { "\n" } else { "" },
			fmt_indentation: if should_pretty_print { "\t" } else { "" },
			fmt_space: if should_pretty_print { " " } else { "" },
		}
	}
	
	fn entry(&mut self) -> Result<String, Error> {
		let entry = self.program.get_main_entry().ok_or_else(|| Error {
			msg: "Cannot execute program without a main function".to_string(),
			span: Span::new((), 0..0),
		})?;
		
		let body = self.block(entry).iter().map(|x| js::format(&x, ";")).collect::<Vec<_>>().join("");
		let functions = self.required_functions.values().map(|x| js::format(x, ";")).collect::<Vec<_>>().join("");
		
		Ok(format!("{}{}", functions, body))
	}
	
	fn block(&mut self, block: &Vec<Entity<'src>>) -> Vec<js::Node<'src>> {
		block.iter().filter_map(|x| self.entity(x)).collect::<Vec<_>>()
	}
	
	fn entity(&mut self, entity: &Entity<'src>) -> Option<js::Node<'src>> {
		Some(match entity {
			Entity::Error => unreachable!(),
			Entity::Value(x) => js::Node::Value(x.clone()),
			Entity::Function(id) => {
				let function = self.program.get_function(id).unwrap();
				self.function(function)
			},
			Entity::Local(id) => {
				js::Node::Local(self.ng.name_for(*id))
			},
			Entity::Call(subject, args) => {
				match &**subject {
					Entity::Local(id) => {
						let args = args.iter().filter_map(|arg| self.entity(arg)).collect::<Vec<_>>();
						if self.program.is_print_fn_id(*id) {
							js::Node::Call(
								Box::new(js::Node::Property(
									Box::new(js::Node::Local("console".to_string())),
									"log".to_string(),
								)),
								args,
							)
						} else {
							if !self.required_functions.contains_key(id) {
								if let Some(function) = self.program.get_function(id) {
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
			Entity::FunctionReturn(value) => js::Node::Return(Box::new(self.entity(value).unwrap_or(js::Node::Void))),
			Entity::Binary(op, lhs, rhs) => js::Node::Binary(op.clone(), Box::new(self.entity(lhs).unwrap_or(js::Node::Void)), Box::new(self.entity(rhs).unwrap_or(js::Node::Void))),
			Entity::Variable(id) => {
				let name = self.ng.name_for(*id);
				let variable = self.program.get_variable(id).unwrap();
				let value = self.entity(&variable.value).unwrap_or(js::Node::Void);
				js::Node::Variable(js::Variable {
					name,
					value: Box::new(value),
				})
			}
		})
	}
	
	fn function(&mut self, function: &Function<'src>) -> js::Node<'src> {
		let name = self.ng.name_for(function.id);
		let parameters = function.parameters.iter().map(|parameter| js::Parameter { name: self.ng.name_for(parameter.id) }).collect::<Vec<_>>();
		let body = self.block(&function.body);
		js::Node::Function(js::Function { name, parameters, body })
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
	
	pub fn format(node: &Node, terminator: &'static str) -> String {
		match node {
			Node::Void => "".to_string(),
			Node::Value(x) => format!("{}{}", x.to_string(), terminator),
			Node::Function(function) => {
				let name = function.name.clone();
				let parameters = function.parameters.iter().map(|x| x.name.clone()).collect::<Vec<_>>().join(",");
				let body = function.body.iter().map(|x| format(x, ";")).collect::<Vec<_>>().join("");
				format!("function {}({}){{{}}}", name, parameters, body)
			}
			Node::Local(name) => format!("{}{}", name, terminator),
			Node::Return(value) => match &**value {
				Node::Void => format!("return{}", terminator),
				x => format!("return {}{}", format(x, ""), terminator),
			},
			Node::Call(subject, args) => {
				let s_subject = format(subject, "");
				let s_args = args.iter().map(|x| format(x, "")).collect::<Vec<_>>().join(",");
				format!("{}({}){}", s_subject, s_args, terminator)
			}
			Node::Binary(op, lhs, rhs) => {
				let s_op = match op {
					BinaryOp::Add => "+",
					BinaryOp::Sub => "+",
					BinaryOp::Mul => "*",
					BinaryOp::Div => "/",
					BinaryOp::Eq => "===",
					BinaryOp::NotEq => "!==",
				};
				format!("{}{}{}{}", format(lhs, ""), s_op, format(rhs, ""), terminator)
			}
			Node::Variable(variable) => {
				let value = format(&variable.value, "");
				format!("const {}={}{}", variable.name, value, terminator)
			}
			Node::Property(subject, member) => {
				let s_subject = format(subject, "");
				format!("{}.{}{}", s_subject, member, terminator)
			}
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
