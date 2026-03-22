
use std::{any::{Any, type_name_of_val}, collections::HashMap};

use crate::{parser::Node, shared::{Span, Spanned, Value}};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityId(u32);

#[derive(Debug)]
pub struct Program<'src> {
	pub root: Entity<'src>,
	pub global_scope: Scope<'src>,
	pub spans: HashMap<EntityId, Span>,
}

impl<'src> Program<'src> {
	pub fn get_main_entry(&self) -> Option<&Entity<'src>> {
		self.global_scope.functions.values().find(|x| x.name == "main").map(|x| x.body.as_ref())
	}
}

pub struct ProgramContext {
	pub next_id: u32,
	pub spans: HashMap<EntityId, Span>,
}

impl ProgramContext {
	pub fn get_next_id(&mut self) -> EntityId {
		let id = self.next_id;
		self.next_id += 1;
		EntityId(id)
	}
}

#[derive(Debug)]
pub struct Scope<'src> {
	pub functions: HashMap<EntityId, Function<'src>>,
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
		next_id: 0,
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
			let id = context.get_next_id();
			context.spans.insert(id, node.1);
			let name = func.name.0;
			let body = analyze_node(func.body.as_ref(), scope, context);
			let function = Function { id, name, parameters: vec![], body: Box::new(body) };
			println!("inserting function");
			scope.functions.insert(id, function);
			Entity::Function(id)
		},
		Node::Call(subject, args) => Entity::Call(
			Box::new(analyze_node(subject, scope, context)),
			args.0.iter().map(|x| analyze_node(x, scope, context)).collect(),
		),
		Node::Local(name) => {
			println!("looking for local '{}'", *name);
			println!("{:#?}", scope);
			let local_id = scope.functions.values().find(|x| x.name == *name).map(|x| x.id);
			Entity::Local(local_id.expect(format!("cannot find '{}'", name).as_str()))
		},
		Node::Seq(children) => {
			let mut nodes = Vec::new();
			
			println!("analyzing sequence");
			
			for child in children {
				match child.0 {
					Node::Func(_) => {
						println!("analyzing function");
						analyze_node(child, scope, context);
					}
					_ => {
						println!("pushing child");
						nodes.push(child);
					}
				}
			}
			
			println!("anaylzing children");
			Entity::Seq(nodes.iter().map(|child| analyze_node(child, scope, context)).collect())
		},
		x => unimplemented!("{x:?}"),
	}
}
