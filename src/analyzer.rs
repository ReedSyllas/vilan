
use std::{any::{Any, type_name_of_val}, collections::HashMap};

use crate::{parser::Node, shared::{Span, Spanned, Value}};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityId(u32);

#[derive(Debug)]
pub struct Program<'src> {
	pub root: Entity<'src>,
	pub global_scope: Scope<'src>,
	context: ProgramContext<'src>,
}

impl<'src> Program<'src> {
	pub fn get_main_entry(&self) -> Option<&Entity<'src>> {
		self.context.functions.values().find(|x| x.name == "main").map(|x| x.body.as_ref())
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
	Error,
	Seq(Vec<Self>),
	Local(EntityId),
	Value(Value<'src>),
	Function(EntityId),
	Call(Box<Self>, Vec<Self>),
}

#[derive(Debug)]
pub struct Function<'src> {
	pub id: EntityId,
	pub name: &'src str,
	pub parameters: Vec<Parameter<'src>>,
	pub body: Box<Entity<'src>>,
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
	pub value: Value<'src>,
}

pub fn analyze<'src>(node: &Spanned<Node<'src>>) -> Program<'src> {
	let mut global_scope = Scope {
		declarations: HashMap::new(),
	};
	let mut context = ProgramContext {
		next_id: 0,
		spans: HashMap::new(),
		functions: HashMap::new(),
		variables: HashMap::new(),
	};
	let root = analyze_node(node, &mut global_scope, &mut context);
	Program {
		root,
		global_scope,
		context,
	}
}

fn analyze_node<'src>(node: &Spanned<Node<'src>>, scope: &mut Scope<'src>, context: &mut ProgramContext<'src>) -> Entity<'src> {
	match &node.0 {
		Node::Value(x) => Entity::Value(x.clone()),
		Node::Func(func) => {
			let name = func.name.0;
			let mut body_scope = Scope {
				declarations: scope.declarations.clone(),
			};
			let parameters = func.parameters.0.iter().map(|(name, _type)| Parameter { id: context.get_next_id(), name }).collect::<Vec<_>>();
			for parameter in parameters.iter() {
				body_scope.declarations.insert(parameter.name, parameter.id);
				let variable = Variable { id: parameter.id, name: parameter.name, value: Value::Null };
				context.variables.insert(parameter.id, variable);
			}
			let body = analyze_node(func.body.as_ref(), &mut body_scope, context);
			let id = scope.declarations.get(name).expect("cannot find id for function '{name}'").clone();
			let function = Function { id, name, parameters, body_scope, body: Box::new(body), call_count: 0 };
			context.functions.insert(id, function);
			Entity::Function(id)
		},
		Node::Call(subject, args) => Entity::Call(
			Box::new(analyze_node(subject, scope, context)),
			args.0.iter().map(|x| analyze_node(x, scope, context)).collect(),
		),
		Node::Local(name) => {
			let local_id = scope.declarations.get(*name).map(|x| *x);
			Entity::Local(local_id.expect(format!("cannot find '{}'", name).as_str()))
		},
		Node::Seq(children) => {
			for child in children {
				analyze_node_discovery(child, scope, context);
			}
			Entity::Seq(
				children
					.iter()
					.map(|child| analyze_node(child, scope, context))
					.collect()
			)
		},
		x => unimplemented!("{x:?}"),
	}
}

fn analyze_node_discovery<'src>(node: &Spanned<Node<'src>>, scope: &mut Scope<'src>, context: &mut ProgramContext) {
	match &node.0 {
		Node::Func(func) => {
			let id = context.get_next_id();
			context.spans.insert(id, node.1);
			let name = func.name.0;
			scope.declarations.insert(name, id);
		}
		_ => (),
	}
}
