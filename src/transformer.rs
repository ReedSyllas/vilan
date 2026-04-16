
use std::{collections::HashMap};
use chumsky::span::Span;

use crate::{analyzer::{Entity, EntityIfBranch, Function, Program}, shared::{BinaryOp, Error, Id}, transformer::js::IfBranch};

pub fn transform<'src>(program: &Program<'src>) -> Result<String, Error> {
	Transformer::new(program, true).transform_entry()
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
			ng: NameGenerator::simple(debug_names),
			required_functions: HashMap::new(),
			formatter: if should_pretty_print { Formatter::new_pretty() } else { Formatter::new_compact() },
		}
	}
	
	fn transform_entry(mut self) -> Result<String, Error> {
		let main_fn_body = self.program.functions.values().find_map(|f| (f.name == "main").then_some(&f.body.0)).ok_or_else(|| Error {
			msg: "Cannot execute program without a main function".to_string(),
			span: Span::new((), 0..0),
		})?;
		
		let body = self.walk_list(main_fn_body).into_iter();
		let functions = self.required_functions.into_values();
		
		Ok(format!("{}{}", self.formatter.file(&functions.chain(body).collect::<Vec<_>>()), self.formatter.line_break))
	}
	
	fn walk_list(&mut self, list: &Vec<Id>) -> Vec<js::Node<'src>> {
		let mut block = Vec::new();
		for item in list {
			if let Some(node) = self.walk_entity(*item, &mut block) {
				block.push(node);
			}
		}
		block
	}
	
	fn walk_entity(&mut self, id: Id, block: &mut Vec<js::Node<'src>>) -> Option<js::Node<'src>> {
		let entity = self.program.entity_map.get(&id).unwrap();
		
		Some(match entity {
			Entity::Error => unreachable!(),
			Entity::Void => js::Node::Void,
			Entity::Null => js::Node::Null,
			Entity::Bool(x) => js::Node::Bool(*x),
			Entity::Number(x) => js::Node::Number(x),
			Entity::String(x) => js::Node::String(x),
			Entity::Struct(_) => {
				return None;
			},
			Entity::Function(id) => {
				let function = self.program.functions.get(id).unwrap();
				self.function(function)
			},
			Entity::Local(id) => {
				js::Node::Local(self.ng.name_for(*id))
			},
			Entity::Member(subject_id, name) => {
				let subject = self.walk_entity(*subject_id, block).unwrap_or(js::Node::Void);
				js::Node::Property(Box::new(subject), name.to_string())
			},
			Entity::Call(id) => {
				let function_call = self.program.function_calls.get(id).unwrap();
				let subject = self.program.entity_map.get(&function_call.subject).unwrap();
				match subject {
					Entity::Local(id) => {
						let args = function_call.arguments.iter().filter_map(|arg| self.walk_entity(*arg, block)).collect::<Vec<_>>();
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
					},
					_ => unimplemented!(),
				}
			},
			Entity::FunctionReturn(value) => js::Node::Return(Box::new(self.walk_entity(*value, block).unwrap_or(js::Node::Void))),
			Entity::Binary(op, lhs, rhs) => js::Node::Binary(*op, Box::new(self.walk_entity(*lhs, block).unwrap_or(js::Node::Void)), Box::new(self.walk_entity(*rhs, block).unwrap_or(js::Node::Void))),
			Entity::Variable(id) => {
				let name = self.ng.name_for(*id);
				if self.program.reference_count.get(id).map(|x| *x < 1).unwrap_or(true) {
					return None;
				}
				let variable = self.program.variables.get(id).unwrap();
				let value = variable.initial.and_then(|id| self.walk_entity(id, block)).unwrap_or(js::Node::Void);
				js::Node::Variable(js::Variable {
					name,
					value: Box::new(value),
				})
			},
			Entity::Block(body) => {
				for statement in &body.0 {
					if let Some(node) = self.walk_entity(*statement, block) {
						block.push(node);
					}
				}
				return self.walk_entity(body.1, block);
			},
			Entity::If(branch) => {
				fn walk_branch<'src>(t: &mut Transformer, branch: &EntityIfBranch, block: &mut Vec<js::Node<'src>>) -> js::IfBranch<'src> {
					match branch {
						EntityIfBranch::If(condition, body, else_) => {
							js::IfBranch::If(t.walk_entity(*condition, block), t.walk_list(body), else_.map(|x| walk_branch(t, x, block)))
						},
						EntityIfBranch::Else(body) => {
							js::IfBranch::Else(t.walk_list(list))
						},
					}
				}
				js::Node::If(walk_branch(self, branch, block))
			},
			Entity::List(items) => {
				js::Node::Void
			},
			Entity::Tuple(ids) => {
				let items = ids.iter().filter_map(|id| self.walk_entity(*id, block)).collect();
				js::Node::Array(items)
			},
			Entity::StructInitializer(struct_id, assignments) => {
				let struct_ = self.program.structs.get(struct_id).unwrap();
				// let mut properties_ng = NameGenerator::simple(debug_names);
				let properties = assignments.iter().filter_map(|(i, id)| {
					let field = struct_.fields.get(*i).unwrap();
					let value = self.walk_entity(*id, block);
					value.map(|x| (field.name, x))
				}).collect::<Vec<_>>();
				js::Node::Object(properties)
			},
		})
	}
	
	fn function(&mut self, function: &Function<'src>) -> js::Node<'src> {
		let name = self.ng.name_for(function.id);
		let parameters = function.parameters.iter().map(|parameter_id| js::Parameter { name: self.ng.name_for(*parameter_id) }).collect::<Vec<_>>();
		let mut body = self.walk_list(&function.body.0);
		if let Some(return_expr) = self.walk_entity(function.body.1, &mut body) {
			body.push(js::Node::Return(Box::new(return_expr)));
		}
		js::Node::Function(js::Function { name, parameters, body })
	}
}

struct Formatter {
	line_break: &'static str,
	indentation: &'static str,
	space: &'static str,
	array_surround: &'static str,
	object_surround: &'static str,
}

impl Formatter {
	fn new_pretty() -> Self {
		Self {
			line_break: "\n",
			indentation: "\t",
			space: " ",
			array_surround: " ",
			object_surround: " ",
		}
	}
	
	fn new_compact() -> Self {
		Self {
			line_break: "",
			indentation: "",
			space: "",
			array_surround: "",
			object_surround: "",
		}
	}
	
	fn file(&self, list: &Vec<js::Node>) -> String {
		self.sequence(list, ";", 0)
	}
	
	fn sequence(&self, list: &Vec<js::Node>, terminator: &'static str, indentation: usize) -> String {
		list.iter().map(|node| self.node(node, terminator, indentation)).collect::<Vec<_>>().join(self.line_break)
	}
	
	fn node(&self, node: &js::Node, terminator: &'static str, indentation: usize) -> String {
		let text = match node {
			js::Node::Void => format!("undefined{}", terminator),
			js::Node::Null => format!("null{}", terminator),
			js::Node::String(x) => format!("\"{}\"{}", x.escape_default(), terminator),
			js::Node::Number(x) => format!("{}{}", x, terminator),
			js::Node::Bool(x) => format!("{}{}", x, terminator),
			js::Node::Array(items) => {
				let s_items = items.iter().map(|x| self.node(x, "", 0)).collect::<Vec<_>>().join(format!(",{}", self.space).as_str());
				format!("[{}{}{}]{}", self.array_surround, s_items, self.array_surround, terminator)
			},
			js::Node::Object(members) => {
				let s_members = members.iter().map(|(key, value)| format!("{}:{}{}", key, self.space, self.node(value, "", 0))).collect::<Vec<_>>().join(format!(",{}", self.space).as_str());
				format!("{{{}{}{}}}{}", self.object_surround, s_members, self.object_surround, terminator)
			},
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
					BinaryOp::Sub => "-",
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
			js::Node::If(branch) => {
				fn walk_branch(f: &Formatter, branch: &IfBranch, indentation: usize) -> String {
					match branch {
						js::IfBranch::If(condition, body, else_) => {
							let s_condition = f.node(condition, "", 0);
							let s_body = body.iter().map(|x| f.node(x, ";", indentation + 1)).collect::<Vec<_>>().join("");
							let s_else = else_.as_ref().map(|x| format!("{}{}", f.space, walk_branch(f, x, indentation))).unwrap_or("".to_string());
							format!("if{}({}){}{{{}}}{}", f.space, s_condition, f.space, s_body, s_else)
						},
						js::IfBranch::Else(body) => {
							let s_body = body.iter().map(|x| f.node(x, ";", indentation + 1)).collect::<Vec<_>>().join("");
							format!("else{}{{{}}}", f.space, s_body)
						},
					}
				}
				walk_branch(self, branch, indentation)
			}
		};
		
		format!("{}{}", self.indentation.repeat(indentation), text)
	}
}

pub mod js {
    use crate::shared::BinaryOp;

	#[derive(Clone, Debug)]
	pub enum Node<'src> {
		Array(Vec<Self>),
		Object(Vec<(&'src str, Self)>),
		Binary(BinaryOp, Box<Self>, Box<Self>),
		Bool(bool),
		Call(Box<Self>, Vec<Self>),
		Function(Function<'src>),
		// TODO: Consider extracting identifiers into a separate lookup table for late identifier substitution.
		Local(String),
		Null,
		Number(&'src str),
		Property(Box<Self>, String),
		Return(Box<Self>),
		String(&'src str),
		Variable(Variable<'src>),
		If(IfBranch<'src>),
		Void,
	}
	
	#[derive(Clone, Debug)]
	pub enum IfBranch<'src> {
		If(Box<Node<'src>>, Vec<Node<'src>>, Option<Box<Self>>),
		Else(Vec<Node<'src>>),
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
	fn new(chars: &str, debug_names: HashMap<Id, String>) -> Self {
		Self {
			chars: chars.chars().collect(),
			counter: 0,
			names: HashMap::new(),
			debug_names,
		}
	}
	
	fn simple(debug_names: HashMap<Id, String>) -> Self {
		Self::new("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ", debug_names)
	}
	
	fn name_for(&mut self, id: Id) -> String {
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
