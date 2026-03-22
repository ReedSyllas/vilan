
use crate::{analyzer::{Entity, Program, Scope}, shared::{BinaryOp, Error, Value}};

pub fn interpret<'src>(program: Program<'src>) -> Result<Value<'src>, Error> {
	interpret_entity(&program.root, &program.global_scope, &program)
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
