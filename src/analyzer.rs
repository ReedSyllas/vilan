
use std::collections::HashMap;
use crate::{parser::Node, shared::{Span, Spanned, Value}};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityId(u32);

#[derive(Debug)]
pub struct Program<'src> {
	pub root: Entity<'src>,
	pub global_scope: Scope<'src>,
	pub spans: HashMap<EntityId, Span>,
}

pub struct ProgramContext {
	pub spans: HashMap<EntityId, Span>,
}

#[derive(Debug)]
pub struct Scope<'src> {
	pub functions: HashMap<EntityId, Function<'src>>,
}

#[derive(Debug)]
pub enum Entity<'src> {
	Error,
	Value(Value<'src>),
	Function(EntityId),
}

#[derive(Debug)]
pub struct Function<'src> {
	pub id: EntityId,
	pub name: &'src str,
	pub parameters: Vec<Parameter<'src>>,
	pub body: Box<Entity<'src>>,
}

#[derive(Debug)]
pub struct Parameter<'src> {
	pub name: &'src str,
	// TODO: Add type support
	// type_: ,
}

pub fn analyze<'src>(node: &Spanned<Node<'src>>) -> Program<'src> {
	let mut global_scope = Scope {
		functions: HashMap::new(),
	};
	let mut context = ProgramContext {
		spans: HashMap::new(),
	};
	let root = analyze_node(node, &mut global_scope, &mut context);
	Program {
		root,
		global_scope,
		spans: context.spans,
	}
}

fn analyze_node<'src>(node: &Spanned<Node<'src>>, scope: &mut Scope<'src>, context: &mut ProgramContext) -> Entity<'src> {
	match &node.0 {
		Node::Value(x) => Entity::Value(x.clone()),
		Node::Func(func) => {
			let id = EntityId(0);
			context.spans.insert(id, node.1);
			
			let name = func.name.0;
			let function = Function { id, name, parameters: vec![], body: Box::new(Entity::Error) };
			scope.functions.insert(id, function);
			Entity::Function(id)
		},
		_ => unimplemented!(),
	}
}
