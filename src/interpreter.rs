
use std::{collections::HashMap};
use crate::{parser::{BinaryOp, Node, Func, Value}, shared::{Error, Spanned}};

pub fn eval_expr<'src>(
	expr: &Spanned<Node<'src>>,
	functions: &HashMap<&'src str, Func<'src>>,
	stack: &mut Vec<(&'src str, Value<'src>)>,
) -> Result<Value<'src>, Error> {
	Ok(match &expr.0 {
		Node::Error => unreachable!(), // Error expressions only get created by parser errors, so cannot exist in a valid AST
		Node::Value(val) => val.clone(),
		Node::List(items) => Value::List(
			items
			.iter()
			.map(|item| eval_expr(item, functions, stack))
			.collect::<Result<_, _>>()?,
		),
		Node::Local(name) =>
			stack
			.iter()
			.rev()
			.find(|(l, _)| l == name)
			.map(|(_, v)| v.clone())
			.or_else(|| Some(Value::Func(name)).filter(|_| functions.contains_key(name)))
			.ok_or_else(|| Error {
				span: expr.1,
				msg: format!("No such variable '{name}' in scope"),
			})?,
		Node::Let(local, val) => {
			let val = eval_expr(val, functions, stack)?;
			stack.push((local, val));
			eval_expr(&(Node::Local(local), expr.1), functions, stack)?
		}
		Node::Import(subjects) => {
			println!("Found import for {subjects:#?}");
			Value::Null
		}
		Node::Then(a, b) => {
			eval_expr(a, functions, stack)?;
			eval_expr(b, functions, stack)?
		}
		Node::Binary(a, BinaryOp::Add, b) => Value::Num(
			eval_expr(a, functions, stack)?.num(a.1)? + eval_expr(b, functions, stack)?.num(b.1)?,
		),
		Node::Binary(a, BinaryOp::Sub, b) => Value::Num(
			eval_expr(a, functions, stack)?.num(a.1)? - eval_expr(b, functions, stack)?.num(b.1)?,
		),
		Node::Binary(a, BinaryOp::Mul, b) => Value::Num(
			eval_expr(a, functions, stack)?.num(a.1)? * eval_expr(b, functions, stack)?.num(b.1)?,
		),
		Node::Binary(a, BinaryOp::Div, b) => Value::Num(
			eval_expr(a, functions, stack)?.num(a.1)? / eval_expr(b, functions, stack)?.num(b.1)?,
		),
		Node::Binary(a, BinaryOp::Eq, b) => {
			Value::Bool(eval_expr(a, functions, stack)? == eval_expr(b, functions, stack)?)
		}
		Node::Binary(a, BinaryOp::NotEq, b) => {
			Value::Bool(eval_expr(a, functions, stack)? != eval_expr(b, functions, stack)?)
		}
		Node::Call(func, args) => {
			let f = eval_expr(func, functions, stack)?;
			match f {
				Value::Func(name) => {
					let f = &functions[&name];
					let mut stack = if f.args.len() != args.0.len() {
						return Err(Error {
							span: expr.1,
							msg: format!("'{}' called with wrong number of arguments (expected {name}, found {})", f.args.len(), args.0.len()),
						});
					} else {
						f.args
						.iter()
						.zip(args.0.iter())
						.map(|(name, arg)| Ok((*name, eval_expr(arg, functions, stack)?)))
						.collect::<Result<_, _>>()?
					};
					eval_expr(&f.body, functions, &mut stack)?
				}
				f => {
					return Err(Error {
						span: func.1,
						msg: format!("'{f:?}' is not callable"),
					})
				}
			}
		}
		Node::If(cond, a, b) => {
			let c = eval_expr(cond, functions, stack)?;
			match c {
				Value::Bool(true) => eval_expr(a, functions, stack)?,
				Value::Bool(false) => eval_expr(b, functions, stack)?,
				c => {
					return Err(Error {
						span: cond.1,
						msg: format!("Conditions must be booleans, found '{c:?}'"),
					})
				}
			}
		}
		Node::Print(a) => {
			let val = eval_expr(a, functions, stack)?;
			println!("{val}");
			val
		}
	})
}
