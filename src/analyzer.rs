
use std::{collections::HashMap};

use crate::{parser::{Node, NodeBlock}, shared::{BinaryOp, Span, Spanned, Value}};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityId(u32);

#[derive(Debug)]
pub struct Program<'src> {
	pub root: Vec<Entity<'src>>,
	pub root_scope: Scope<'src>,
	context: ProgramContext<'src>,
}

impl<'src> Program<'src> {
	pub fn get_main_entry(&self) -> Option<&Vec<Entity<'src>>> {
		self.context.functions.values().find(|x| x.name == "main").map(|x| &x.body)
	}
	
	pub fn get_function(&self, id: &EntityId) -> Option<&Function<'src>> {
		self.context.functions.get(id)
	}
	
	pub fn get_variable(&self, id: &EntityId) -> Option<&Variable<'src>> {
		self.context.variables.get(id)
	}
}

#[derive(Debug)]
struct ProgramContext<'src> {
	next_id: u32,
	spans: HashMap<EntityId, Span>,
	functions: HashMap<EntityId, Function<'src>>,
	variables: HashMap<EntityId, Variable<'src>>,
}

impl<'src> ProgramContext<'src> {
	fn new() -> Self {
		Self {
			next_id: 0,
			spans: HashMap::new(),
			functions: HashMap::new(),
			variables: HashMap::new(),
		}
	}
	
	pub fn get_next_id(&mut self) -> EntityId {
		let id = self.next_id;
		self.next_id += 1;
		EntityId(id)
	}
}

#[derive(Debug)]
pub struct Scope<'src> {
	pub declarations: HashMap<&'src str, EntityId>,
}

#[derive(Clone, Debug)]
pub enum Entity<'src> {
	Binary(BinaryOp, Box<Self>, Box<Self>),
	Call(Box<Self>, Vec<Self>),
	Error,
	Function(EntityId),
	FunctionReturn(Box<Self>),
	Local(EntityId),
	Variable(EntityId),
	Value(Value<'src>),
}

#[derive(Debug)]
pub struct Function<'src> {
	pub id: EntityId,
	pub name: &'src str,
	pub parameters: Vec<Parameter<'src>>,
	pub body: Vec<Entity<'src>>,
	pub body_scope: Scope<'src>,
	pub call_count: u32,
}

#[derive(Debug)]
pub struct Parameter<'src> {
	pub id: EntityId,
	pub name: &'src str,
	// TODO: Add type support
	// type_: ,
}

#[derive(Debug)]
pub struct Variable<'src> {
	pub id: EntityId,
	pub name: &'src str,
	pub value: Entity<'src>,
}

#[derive(Debug)]
pub struct Analyzer<'src> {
	context: ProgramContext<'src>,
}

impl<'src> Analyzer<'src> {
	fn new() -> Self {
		Self {
			context: ProgramContext::new(),
		}
	}
	
	fn file(mut self, block: &Spanned<NodeBlock<'src>>) -> Program<'src> {
		let mut root_scope = Scope {
			declarations: HashMap::new(),
		};
		let root = self.block(block, &mut root_scope);
		Program {
			root,
			root_scope,
			context: self.context,
		}
	}
	
	fn block(&mut self, block: &Spanned<NodeBlock<'src>>, scope: &mut Scope<'src>) -> Vec<Entity<'src>> {
		for child in block.0.iter() {
			self.node_discovery(&child, scope);
		}
		
		block.0.iter().map(|child| self.node(child, scope)).collect::<Vec<_>>()
	}
	
	fn node(&mut self, node: &Spanned<Node<'src>>, scope: &mut Scope<'src>) -> Entity<'src> {
		match &node.0 {
			Node::Value(x) => Entity::Value(x.clone()),
			Node::Func(func) => {
				let name = func.name.0;
				let mut body_scope = Scope {
					declarations: scope.declarations.clone(),
				};
				let parameters = func.parameters.0.iter().map(|(name, _type)| Parameter { id: self.context.get_next_id(), name }).collect::<Vec<_>>();
				for parameter in parameters.iter() {
					body_scope.declarations.insert(parameter.name, parameter.id);
					let variable = Variable { id: parameter.id, name: parameter.name, value: Entity::Value(Value::Null) };
					self.context.variables.insert(parameter.id, variable);
				}
				let body = self.block(&func.body, &mut body_scope);
				let id = scope.declarations.get(name).expect("cannot find id for function '{name}'").clone();
				let function = Function { id, name, parameters, body_scope, body, call_count: 0 };
				self.context.functions.insert(id, function);
				Entity::Function(id)
			},
			Node::Call(subject, args) => Entity::Call(
				Box::new(self.node(subject, scope)),
				args.0.iter().map(|x| self.node(x, scope)).collect(),
			),
			Node::Local(name) => {
				let local_id = scope.declarations.get(*name).map(|x| *x);
				Entity::Local(local_id.expect(format!("cannot find '{}'", name).as_str()))
			},
			Node::FuncReturn(value) => Entity::FunctionReturn(Box::new(self.node(value, scope))),
			Node::Binary(op, lhs, rhs) => Entity::Binary(op.clone(), Box::new(self.node(lhs, scope)), Box::new(self.node(rhs, scope))),
			Node::Let(name, value) => {
				let id = self.context.get_next_id();
				let e_value = self.node(value, scope);
				self.context.variables.insert(id, Variable {
					id,
					name,
					value: e_value,
				});
				scope.declarations.insert(name, id);
				Entity::Variable(id)
			}
			x => unimplemented!("{x:?}"),
		}
	}
	
	fn node_discovery(&mut self, node: &Spanned<Node<'src>>, scope: &mut Scope<'src>) {
		match &node.0 {
			Node::Func(func) => {
				let id = self.context.get_next_id();
				self.context.spans.insert(id, node.1);
				let name = func.name.0;
				scope.declarations.insert(name, id);
			}
			_ => {}
		}
	}
}

pub fn analyze<'src>(block: &Spanned<NodeBlock<'src>>) -> Program<'src> {
	Analyzer::new().file(block)
}
