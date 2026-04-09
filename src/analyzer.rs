
use std::{collections::HashMap, hash::Hash};

use crate::{parser::{Node, NodeList}, shared::{BinaryOp, PrimitiveType, Span, Spanned, Type, Value}};

#[derive(Clone, Debug)]
pub enum Entity<'src> {
	Binary(BinaryOp, Id, Id),
	If(Id, (Vec<Id>, Id), Option<(Vec<Id>, Id)>),
	Call(Id, Vec<Id>),
	Error,
	Function(Id),
	FunctionReturn(Id),
	Local(Id),
	Value(Value<'src>),
	List(Vec<Id>),
	Block((Vec<Id>, Id)),
	Void,
}

#[derive(Debug)]
pub struct Function<'src> {
	pub id: Id,
	pub name: &'src str,
	pub parameters: Vec<Parameter<'src>>,
	pub body: (Vec<Id>, Id, usize),
	pub call_count: u32,
}

#[derive(Debug)]
pub struct Parameter<'src> {
	pub name: &'src str,
	pub type_: Type,
	// TODO: Add type support
}

#[derive(Debug)]
pub struct Variable<'src> {
	pub id: Id,
	pub name: &'src str,
	pub initial: Option<Id>,
	pub type_: Type,
	// pub mutable: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Id(u32);

impl std::fmt::Debug for Id {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "Id({})", self.0)
	}
}

#[derive(Debug)]
pub struct Scope<'src> {
	pub name_id_map: HashMap<&'src str, Id>,
}

impl<'src> Scope<'src> {
	fn create_child(&self) -> Scope<'src> {
		Scope {
			name_id_map: self.name_id_map.clone()
		}
	}
}

#[derive(Debug)]
pub struct Analyzer<'src> {
	assignment_values: HashMap<Id, Vec<Id>>,
	entity_map: HashMap<Id, Entity<'src>>,
	locals: Vec<(Id, &'src str)>,
	next_id: u32,
	reference_count: HashMap<Id, u32>,
	scope_map: HashMap<Id, usize>,
	scopes: Vec<Scope<'src>>,
	span_map: HashMap<Id, &'src Span>,
	variables: HashMap<Id, Variable<'src>>,
	functions: HashMap<Id, Function<'src>>,
}

impl<'src> Analyzer<'src> {
	fn new() -> Self {
		Self {
			assignment_values: HashMap::new(),
			entity_map: HashMap::new(),
			locals: Vec::new(),
			next_id: 0,
			reference_count: HashMap::new(),
			scope_map: HashMap::new(),
			scopes: Vec::new(),
			span_map: HashMap::new(),
			variables: HashMap::new(),
			functions: HashMap::new(),
		}
	}
	
	fn get_next_id(&mut self) -> Id {
		let id = self.next_id;
		self.next_id += 1;
		Id(id)
	}
	
	fn get_entity_by_id(&self, id: Id) -> &Entity<'src> {
		self.entity_map.get(&id).expect("failed to get entity for id")
	}
	
	fn get_scope_for_node(&mut self, id: Id) -> &mut Scope<'src> {
		self.scope_map.get(&id).map(|idx| *idx).map(|idx| self.get_scope_by_idx(idx)).expect("failed to get scope for node")
	}
	
	fn get_scope_by_idx(&mut self, idx: usize) -> &mut Scope<'src> {
		self.scopes.get_mut(idx).expect("failed to get scope for idx")
	}
	
	fn push_scope(&mut self, scope: Scope<'src>) -> usize {
		self.scopes.push(scope);
		self.scopes.len() - 1
	}
	
	fn walk_list(&mut self, list: &'src NodeList<'src>, scope_idx: usize) -> Vec<Id> {
		list.iter().map(|child| self.walk_node(child, scope_idx)).collect::<Vec<_>>()
	}
	
	fn walk_node(&mut self, node: &'src Spanned<Node<'src>>, scope_idx: usize) -> Id {
		let id = self.get_next_id();
		
		let entity = match &node.0 {
			Node::Error => Some(Entity::Error),
			Node::Void => Some(Entity::Void),
			Node::Local(name) => {
				self.locals.push((id, name));
				None
			},
			Node::Import(_) => None,
			Node::Value(x) => Some(Entity::Value(x.clone())),
			Node::List(items) => {
				let ids = self.walk_list(items, scope_idx);
				Some(Entity::List(ids))
			},
			Node::Block(children) => {
				let body_scope = self.get_scope_by_idx(scope_idx).create_child();
				let body_scope_idx = self.push_scope(body_scope);
				let ids = self.walk_list(&children.0.0, body_scope_idx);
				let expr_id = self.walk_node(&children.0.1, body_scope_idx);
				Some(Entity::Block((ids, expr_id)))
			},
			Node::If(if_) => {
				let body_scope = self.get_scope_by_idx(scope_idx).create_child();
				let body_scope_idx = self.push_scope(body_scope);
				let condition_id = self.walk_node(&if_.condition, body_scope_idx);
				let then_ids = self.walk_list(&if_.then.0.0, body_scope_idx);
				let then_expr_id = self.walk_node(&if_.then.0.1, body_scope_idx);
				Some(Entity::If(condition_id, (then_ids, then_expr_id), None))
			},
			Node::Func(function) => {
				let name = function.name.0;
				let parameters = function.parameters.0.iter().map(|x| Parameter { name: x.0, type_: x.1.as_ref().map(|x| self.walk_type(x)).unwrap_or(Type::Unknown) }).collect::<Vec<_>>();
				let body_scope = self.get_scope_by_idx(scope_idx).create_child();
				let body_scope_idx = self.push_scope(body_scope);
				let ids = self.walk_list(&function.body.0.0, body_scope_idx);
				let expr_id = self.walk_node(&function.body.0.1, body_scope_idx);
				self.functions.insert(id, Function { id, name, parameters, body: (ids, expr_id, body_scope_idx), call_count: 0 });
				Some(Entity::Function(id))
			},
			Node::Call(subject, arguments) => {
				let subject_id = self.walk_node(subject, scope_idx);
				let argument_ids = self.walk_list(&arguments.0, scope_idx);
				Some(Entity::Call(subject_id, argument_ids))
			},
			Node::FuncReturn(value) => {
				let id = self.walk_node(value, scope_idx);
				Some(Entity::FunctionReturn(id))
			},
			Node::Binary(op, lhs, rhs) => {
				let lhs_id = self.walk_node(lhs, scope_idx);
				let rhs_id = self.walk_node(rhs, scope_idx);
				Some(Entity::Binary(*op, lhs_id, rhs_id))
			},
			Node::Let(name, type_, value) => {
				let scope = self.get_scope_by_idx(scope_idx);
				scope.name_id_map.insert(name, id);
				self.reference_count.entry(id).or_insert(0);
				let initial = value.as_ref().map(|value| {
					let value_id = self.walk_node(value, scope_idx);
					let assignments = self.assignment_values.entry(id).or_insert_with(|| Vec::new());
					assignments.push(value_id);
					value_id
				});
				let type_ = type_.as_ref().map(|x| self.walk_type(x)).unwrap_or(Type::Unknown);
				self.variables.insert(id, Variable { id, name, initial, type_ });
				Some(Entity::Local(id))
			},
		};
		
		if let Some(entity) = entity {
			self.entity_map.insert(id, entity);
		}
		
		self.span_map.insert(id, &node.1);
		self.scope_map.insert(id, scope_idx);
		
		id
	}
	
	fn walk_type(&mut self, node: &'src Spanned<Node<'src>>) -> Type {
		match node.0 {
			Node::Local(name) => match name {
				"f64" => Type::Primitive(PrimitiveType::F64),
				"i32" => Type::Primitive(PrimitiveType::I32),
				"u32" => Type::Primitive(PrimitiveType::U32),
				"bool" => Type::Primitive(PrimitiveType::Bool),
				"null" => Type::Primitive(PrimitiveType::Null),
				"str" => Type::Primitive(PrimitiveType::String),
				_ => Type::Unknown,
			},
			_ => Type::Unknown,
		}
	}
	
	fn resolve_type(&self, id: Id, constraint: Type) -> Type {
		constraint.clone().reconcile(match self.get_entity_by_id(id) {
			Entity::Value(value) => match value {
				Value::Bool(_) => Type::Primitive(PrimitiveType::Bool),
				Value::Func(_) => Type::Void,
				Value::Interrupt(_) => Type::Interrupt,
				Value::List(_) => Type::Primitive(PrimitiveType::List(Box::new(Type::Void))),
				Value::Null => Type::Primitive(PrimitiveType::Null),
				Value::Num(_) => Type::Primitive(match constraint {
					Type::Primitive(PrimitiveType::F64) => PrimitiveType::F64,
					Type::Primitive(PrimitiveType::U32) => PrimitiveType::U32,
					_ => PrimitiveType::I32,
				}),
				Value::Str(_) => Type::Primitive(PrimitiveType::String),
			},
			_ => Type::Void,
		})
	}
	
	fn build(&mut self) {
		for (id, name) in self.locals.clone() {
			let scope = self.get_scope_for_node(id);
			let subject_id = *scope.name_id_map.get(name).expect(format!("cannot find '{}'", name).as_ref());
			if let Some(rc) = self.reference_count.get_mut(&subject_id) {
				*rc += 1;
			}
			self.entity_map.insert(id, Entity::Local(subject_id));
		}
		
		for (variable_id, value_ids) in self.assignment_values.clone() {
			if let Some(variable) = self.variables.get(&variable_id) {
				let mut type_ = variable.type_.clone();
				
				for value_id in value_ids {
					type_ = self.resolve_type(value_id, type_);
				}
				
				self.variables.get_mut(&variable_id).unwrap().type_ = type_;
			}
		}
	}
}

#[derive(Debug)]
pub struct Program<'src> {
	span_map: HashMap<Id, &'src Span>,
	entity_map: HashMap<Id, Entity<'src>>,
	scope_map: HashMap<Id, usize>,
	scopes: Vec<Scope<'src>>,
	reference_count: HashMap<Id, u32>,
	print_fn_id: Id,
	variables: HashMap<Id, Variable<'src>>,
	functions: HashMap<Id, Function<'src>>,
}

pub fn analyze<'src>(nodes: &'src Spanned<NodeList<'src>>) -> Program<'src> {
	let mut analyzer = Analyzer::new();
	let mut root_scope = Scope {
		name_id_map: HashMap::new(),
	};
	let print_fn_id = analyzer.get_next_id();
	root_scope.name_id_map.insert("print", print_fn_id);
	let root_scope_idx = analyzer.push_scope(root_scope);
	analyzer.walk_list(&nodes.0, root_scope_idx);
	analyzer.build();
	Program {
		span_map: analyzer.span_map,
		entity_map: analyzer.entity_map,
		scope_map: analyzer.scope_map,
		scopes: analyzer.scopes,
		reference_count: analyzer.reference_count,
		print_fn_id,
		variables: analyzer.variables,
		functions: analyzer.functions,
	}
}
