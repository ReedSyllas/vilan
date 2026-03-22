
use chumsky::span::Span;

use crate::{analyzer::{Entity, Program, Scope}, shared::{BinaryOp, Error, Value}};

pub fn interpret<'src>(program: Program<'src>) -> Result<Value<'src>, Error> {
	let entry = program.get_main_entry();
	entry
	.ok_or_else(|| Error {
		msg: "Cannot execute program without a main function".to_string(),
		span: Span::new((), 0..0),
	})
	.and_then(|x| interpret_entity(x, &program.global_scope, &program))
}

fn interpret_entity<'src>(entity: &Entity<'src>, scope: &Scope<'src>, _program: &Program<'src>) -> Result<Value<'src>, Error> {
	Ok(match entity {
		Entity::Error => unreachable!(),
		Entity::Value(val) => val.clone(),
		Entity::Function(id) => {
			println!("Found a function with id {:#?} and name {:#?}", id, scope.functions.get(id).map(|x| x.name));
			Value::Null
		}
		_ => unimplemented!(),
	})
}
