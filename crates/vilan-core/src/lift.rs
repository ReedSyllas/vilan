//! The expression-lifting region rewrite (proposal/expression-lifting.md).
//!
//! A bare postfix `?` parses as a `Node::Lifted` mark; this pass — pure
//! Node → Node, run once per parsed tree before analysis — turns every marked
//! slot-root expression into a `Node::LiftRegion`: an ordered step list
//! (`Eval` steps hoisting effect-capable material that precedes a later `?`,
//! so source evaluation order holds; `Split` steps for the receivers) over a
//! residual body skeleton whose `LiftHole(i)` nodes reference step results.
//!
//! The slot rules (§2/§6 of the proposal): a region never crosses a slot
//! boundary — a `let` initializer, a call argument, an index expression, a
//! struct field, a list/tuple element, a condition, a match subject, a branch
//! or block tail each root their own region — and a recorded paren group
//! (`Node::LiftGroup`) seals its interior as a region of its own. Chain-form
//! `?.` lifts (`Node::Lift`) are sealed atoms: their value participates in a
//! region container-typed, never absorbed (§5 — `a?.b == None` keeps its
//! meaning).
//!
//! A mark in a position this pass does not handle is left in place; the
//! analyzer's walk reports it as a clean "not supported here" diagnostic —
//! sound by construction, never silent.

use crate::node::{Closure, If, Node, NodeIfBranch, NodeList};
use crate::span::Spanned;

/// Rewrite every marked region in a parsed tree, in place. The single entry —
/// called on the entry file's items, each loaded module's items, and parsed
/// macro-expansion output, right where each tree is still owned.
pub fn rewrite_items(items: &mut NodeList<'_>) {
    for item in items.iter_mut() {
        if item.0.contains_lift_mark() {
            take_and_seal(item);
        }
    }
}

fn take_and_seal(slot: &mut Spanned<Node<'_>>) {
    let span = slot.1;
    let owned = std::mem::replace(&mut slot.0, Node::Error);
    *slot = seal((owned, span));
}

/// Process a node as a region ROOT (a slot). If its flat extent carries `?`
/// marks, linearize it into a `LiftRegion`; otherwise recurse into its
/// children, each of which roots its own region.
fn seal<'src>(node: Spanned<Node<'src>>) -> Spanned<Node<'src>> {
    if !node.0.contains_lift_mark() {
        return node;
    }
    let mut remaining = count_direct_splits(&node.0);
    if remaining == 0 {
        return descend(node);
    }
    let span = node.1;
    let mut steps = Vec::new();
    let body = linearize(node, &mut steps, &mut remaining);
    (Node::LiftRegion(steps, Box::new(body)), span)
}

fn seal_boxed<'src>(node: Box<Spanned<Node<'src>>>) -> Box<Spanned<Node<'src>>> {
    Box::new(seal(*node))
}

fn seal_opt<'src>(node: Option<Box<Spanned<Node<'src>>>>) -> Option<Box<Spanned<Node<'src>>>> {
    node.map(seal_boxed)
}

fn seal_list(list: &mut NodeList<'_>) {
    for item in list.iter_mut() {
        if item.0.contains_lift_mark() {
            take_and_seal(item);
        }
    }
}

fn seal_body(body: &mut (NodeList<'_>, Box<Spanned<Node<'_>>>)) {
    seal_list(&mut body.0);
    take_and_seal(&mut body.1);
}

/// How many `?` marks are reachable through the FLAT extent of this node —
/// the operator/postfix material that shares a region with it. Slots (call
/// arguments, indices, elements, fields, control-flow interiors), recorded
/// paren groups, and chain-form lifts delimit the extent.
fn count_direct_splits(node: &Node) -> usize {
    match node {
        Node::Lifted(inner) => 1 + count_direct_splits(&inner.0),
        Node::Binary(_, left, right) => {
            count_direct_splits(&left.0) + count_direct_splits(&right.0)
        }
        Node::Unary(_, inner)
        | Node::Reference(_, inner)
        | Node::Dereference(inner)
        | Node::TryAssert(inner)
        | Node::Await(inner) => count_direct_splits(&inner.0),
        Node::MemberAccessor(subject, _) => count_direct_splits(&subject.0),
        // A call's or subscript's SUBJECT is flat; the arguments / the index
        // are slots.
        Node::Call(subject, _, _) | Node::Index(subject, _) => count_direct_splits(&subject.0),
        _ => 0,
    }
}

/// Linearize a node known to carry direct splits: emit its evaluation-order
/// steps into `steps` and return the body skeleton. `remaining` counts the
/// splits not yet emitted — hoisting only matters while a later split exists
/// (material after the last split evaluates in the final good branch, which
/// is already source order).
fn linearize<'src>(
    node: Spanned<Node<'src>>,
    steps: &mut Vec<(Spanned<Node<'src>>, bool)>,
    remaining: &mut usize,
) -> Spanned<Node<'src>> {
    let (kind, span) = node;
    match kind {
        Node::Lifted(inner) => {
            // Marks inside the receiver expression linearize first (they
            // evaluate before it splits) — but the receiver itself needs no
            // eval-hoist: it IS its split step, evaluated at exactly this
            // position in the step order.
            let subject = {
                let inner = *inner;
                if count_direct_splits(&inner.0) > 0 {
                    linearize(inner, steps, remaining)
                } else if inner.0.contains_lift_mark() {
                    descend(inner)
                } else {
                    inner
                }
            };
            *remaining -= 1;
            steps.push((subject, true));
            (Node::LiftHole(steps.len() - 1), span)
        }
        Node::Binary(op, left, right) => {
            let left = flat_operand(*left, steps, remaining);
            let right = flat_operand(*right, steps, remaining);
            (Node::Binary(op, Box::new(left), Box::new(right)), span)
        }
        Node::Unary(op, inner) => {
            let inner = flat_operand(*inner, steps, remaining);
            (Node::Unary(op, Box::new(inner)), span)
        }
        Node::Reference(mutable, inner) => {
            let inner = flat_operand(*inner, steps, remaining);
            (Node::Reference(mutable, Box::new(inner)), span)
        }
        Node::Dereference(inner) => {
            let inner = flat_operand(*inner, steps, remaining);
            (Node::Dereference(Box::new(inner)), span)
        }
        Node::TryAssert(inner) => {
            // Kept in the skeleton; the walk rejects a `!` that would run
            // after a split (it may not early-return from inside a region).
            let inner = flat_operand(*inner, steps, remaining);
            (Node::TryAssert(Box::new(inner)), span)
        }
        Node::Await(inner) => {
            let inner = flat_operand(*inner, steps, remaining);
            (Node::Await(Box::new(inner)), span)
        }
        Node::MemberAccessor(subject, member) => {
            let subject = flat_operand(*subject, steps, remaining);
            (Node::MemberAccessor(Box::new(subject), member), span)
        }
        Node::Call(subject, generics, mut arguments) => {
            let subject = flat_operand(*subject, steps, remaining);
            // Arguments are slots — and they evaluate after the subject's
            // split, i.e. only on the good path (lazy right).
            seal_list(&mut arguments.0);
            (Node::Call(Box::new(subject), generics, arguments), span)
        }
        Node::Index(subject, index) => {
            let subject = flat_operand(*subject, steps, remaining);
            let index = seal_boxed(index);
            (Node::Index(Box::new(subject), index), span)
        }
        // Only kinds `count_direct_splits` traverses can reach here.
        other => unreachable!(
            "linearize reached a non-flat node kind: {:?}",
            std::mem::discriminant(&other)
        ),
    }
}

/// One operand of the flat extent. A side that still carries direct splits
/// linearizes; a side whose marks all sit under sealed sub-slots (or that has
/// none) is a maximal split-free unit — hoisted into an `Eval` step while a
/// later split exists, so it cannot be reordered after that split's branch,
/// unless it is order-immune (a literal).
fn flat_operand<'src>(
    node: Spanned<Node<'src>>,
    steps: &mut Vec<(Spanned<Node<'src>>, bool)>,
    remaining: &mut usize,
) -> Spanned<Node<'src>> {
    if count_direct_splits(&node.0) > 0 {
        return linearize(node, steps, remaining);
    }
    // Marks under this operand's own slots (a paren group, a call argument,
    // a branch) seal where they sit; the operand then travels as one unit.
    let node = if node.0.contains_lift_mark() {
        descend(node)
    } else {
        node
    };
    if *remaining == 0 || is_order_immune(&node.0) {
        return node;
    }
    let span = node.1;
    steps.push((node, false));
    (Node::LiftHole(steps.len() - 1), span)
}

/// Values whose evaluation cannot be observed out of order — safe to leave in
/// the skeleton even before a split. Everything else (identifiers included:
/// a later receiver could mutate what they name) hoists.
fn is_order_immune(node: &Node) -> bool {
    matches!(
        node,
        Node::Number(..)
            | Node::String(_)
            | Node::MultilineString(_)
            | Node::Bool(_)
            | Node::Null
            | Node::Void
    )
}

/// Recurse into a node whose marks all live below its own slot boundaries:
/// rebuild it with each interior expression sealed as its own region root.
/// Kinds not handled here keep their marks, which the analyzer's walk reports
/// as a clean unsupported-position diagnostic.
fn descend<'src>(node: Spanned<Node<'src>>) -> Spanned<Node<'src>> {
    let (kind, span) = node;
    let kind = match kind {
        // A recorded paren group: its interior is a region root, and the
        // wrapper's job is done.
        Node::LiftGroup(inner) => return seal(*inner),
        Node::Block(mut body) => {
            seal_body(&mut body.0);
            Node::Block(body)
        }
        Node::If(branch) => Node::If(descend_if(branch)),
        Node::Match(subject, mut legs) => {
            let subject = seal_boxed(subject);
            for (patterns, guard, body) in legs.0.iter_mut() {
                let _ = patterns;
                if let Some(guard) = guard {
                    take_and_seal(guard);
                }
                take_and_seal(body);
            }
            Node::Match(subject, legs)
        }
        Node::For(condition, mut body) => {
            seal_body(&mut body.0);
            Node::For(condition.map(seal_boxed), body)
        }
        Node::ForIn(binding, iterable, mut body) => {
            let iterable = seal_boxed(iterable);
            seal_body(&mut body.0);
            Node::ForIn(binding, iterable, body)
        }
        Node::Func(mut function) => {
            if let Some(body) = function.body.as_mut() {
                seal_body(&mut body.0);
            }
            Node::Func(function)
        }
        Node::Closure(Closure {
            parameters,
            return_type,
            return_value,
        }) => Node::Closure(Closure {
            parameters,
            return_type,
            return_value: seal_boxed(return_value),
        }),
        Node::Let(name, annotation, value, mutable) => {
            Node::Let(name, annotation, seal_opt(value), mutable)
        }
        Node::LetDestructure(pattern, annotation, value, mutable) => {
            Node::LetDestructure(pattern, annotation, seal_opt(value), mutable)
        }
        Node::Assign(target, op, value) => Node::Assign(seal_boxed(target), op, seal_boxed(value)),
        Node::Call(subject, generics, mut arguments) => {
            let subject = seal_boxed(subject);
            seal_list(&mut arguments.0);
            Node::Call(subject, generics, arguments)
        }
        Node::Index(subject, index) => Node::Index(seal_boxed(subject), seal_boxed(index)),
        Node::MemberAccessor(subject, member) => Node::MemberAccessor(seal_boxed(subject), member),
        Node::List(mut items) => {
            seal_list(&mut items);
            Node::List(items)
        }
        Node::Tuple(mut items) => {
            seal_list(&mut items);
            Node::Tuple(items)
        }
        Node::Repeat(value, length) => Node::Repeat(seal_boxed(value), seal_boxed(length)),
        Node::StructInitializer(name, generics, mut fields) => {
            for field in fields.0.iter_mut() {
                if let Some(value) = field.0.1.as_mut() {
                    take_and_seal(value);
                }
            }
            Node::StructInitializer(name, generics, fields)
        }
        Node::Binary(op, left, right) => Node::Binary(op, seal_boxed(left), seal_boxed(right)),
        Node::Unary(op, inner) => Node::Unary(op, seal_boxed(inner)),
        Node::Reference(mutable, inner) => Node::Reference(mutable, seal_boxed(inner)),
        Node::Dereference(inner) => Node::Dereference(seal_boxed(inner)),
        Node::TryAssert(inner) => Node::TryAssert(seal_boxed(inner)),
        Node::Await(inner) => Node::Await(seal_boxed(inner)),
        Node::Async(inner) => Node::Async(seal_boxed(inner)),
        Node::FuncReturn(value) => Node::FuncReturn(seal_opt(value)),
        Node::Export(inner) => Node::Export(seal_boxed(inner)),
        Node::Const(inner) => Node::Const(seal_boxed(inner)),
        Node::Derive(names, inner) => Node::Derive(names, seal_boxed(inner)),
        Node::Service(name, inner) => Node::Service(name, seal_boxed(inner)),
        Node::MacroAttribute(name, name_span, arguments, inner) => {
            Node::MacroAttribute(name, name_span, arguments, seal_boxed(inner))
        }
        Node::Module(name, mut items) => {
            seal_list(&mut items.0);
            Node::Module(name, items)
        }
        Node::Impl(subject, traits, mut members) => {
            seal_list(&mut members.0);
            Node::Impl(subject, traits, members)
        }
        Node::Trait(name, generics, supertraits, mut members) => {
            seal_list(&mut members.0);
            Node::Trait(name, generics, supertraits, members)
        }
        // A chain-form lift is a sealed atom: its own value never absorbs
        // into a region (§5), but marks in its interior slots (arguments of
        // a continuation call, a nested subject) still seal in place.
        Node::Lift(subject, continuation) => {
            Node::Lift(seal_boxed(subject), seal_boxed(continuation))
        }
        Node::Is(subject, pattern) => Node::Is(seal_boxed(subject), pattern),
        Node::TupleComprehension {
            binder,
            binder_span,
            source,
            body,
        } => Node::TupleComprehension {
            binder,
            binder_span,
            source: seal_boxed(source),
            body: seal_boxed(body),
        },
        // Everything else either cannot contain an expression (leaves,
        // types, declarations without expression positions) or is a position
        // v1 does not lift in — the mark survives and the walk reports it.
        other => other,
    };
    (kind, span)
}

fn descend_if(branch: NodeIfBranch<'_>) -> NodeIfBranch<'_> {
    match branch {
        NodeIfBranch::If(if_) => {
            let If {
                condition,
                mut then,
                else_,
            } = *if_;
            let condition = seal_boxed(condition);
            seal_body(&mut then.0);
            let else_ = else_.map(|(inner, span)| (descend_if(inner), span));
            NodeIfBranch::If(Box::new(If {
                condition,
                then,
                else_,
            }))
        }
        NodeIfBranch::Else(mut body) => {
            seal_body(&mut body.0);
            NodeIfBranch::Else(body)
        }
    }
}
