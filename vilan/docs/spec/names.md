# Spec §4 — Names, modules, and packages

## 4.1 Modules

A module is one source file. Its top-level statements form its body; its
declarations (`fun`, `struct`, `enum`, `trait`, `impl`, module-level
`let`, `mod` blocks) are its items. There is no separate module
declaration — a file `routes.vl` in a package's source root is the module
`routes` of that package.

## 4.2 The three namespaces

A path's first segment selects a namespace:

- `std::name` — the standard library module `name`, resolved against the
  std package's layers for the current platform (§11).
- `pkg::name` — the module `name` of the **importing file's own
  package**.
- `depname::name` — the module `name` of the dependency declared as
  `depname` in the package manifest.

Within std itself, sibling modules are referenced as `pkg::…` (std is its
own package). A module name that resolves both as `name.vl` and
`name/lib.vl` is an **ambiguity error**.

## 4.3 Imports

`import path` (§3.2) loads the target module (once per program — loading
is idempotent and cycle-tolerant) and binds the imported items in the
importing module's scope:

- `import std::print;` — binds the item `print`.
- `import std::reactive::{ Signal, combine };` — binds each set member.
- `import std::option::Option::{ self, Some, None };` — a path into a
  TYPE: `self` binds the type itself; variant names bind the variants for
  unqualified use.

`use path` binds names from an already-visible type's namespace without
loading (variants, statics). `export statement` re-exports: importers of
this module see the exported names as if declared here.

Platform gating is checked at the import: importing a module outside the
current platform's std layers (e.g. `std::ui` in a node build) is a
compile error naming the platform (§11).

## 4.4 Scopes and shadowing

Scopes nest: module → function/impl → block → closure. Name lookup walks
outward from the use site to the innermost binding. A `let`/`mut` binding
**shadows** any outer binding of the same name from its point of
declaration onward, including imports and items:

```vilan
import std::print;

fun main() {
	let print_count = 2;
	mut label = "a";
	{
		let label = "inner";     // shadows the outer binding in this block
		print(label);
	}
	print(label);
	print(print_count);
}
```

Items within one module share the module scope and are visible
**throughout** the module regardless of declaration order (a function may
call one declared later). Local `let` bindings are visible only after
their declaration.

## 4.5 Type position vs value position

A name is resolved differently by position:

- In **type position** (annotations, generic arguments, impl subjects),
  lookup prefers bindings that denote types; a value binding with the same
  name does not shadow a type there.
- In **value position**, lookup takes the nearest binding of any kind.

Consequently a local variable named `Signal` does not break `let s:
Signal<i32>` annotations in the same scope — but relying on this is poor
style.

## 4.6 Statics and members

`Type::member` (§3.6) resolves `member` in `Type`'s namespace: enum
variants, and the static functions of the type's impls (those without
`self`). `value.member` resolves against the value's type: fields first,
then methods of inherent impls, then trait members visible via the type's
impls (§5.7). Generic statics take their arguments at the path head:
`List<str>::new()`.

## 4.7 The prelude

A small set of names is in scope without imports: the primitive types
(`i32`, `str`, `bool`, …), `List`, `void`, and the boolean/`null`
literals' types. Everything else — including `Option`, `Result`, `print`
— must be imported. (The exact prelude is the lang-item table, appendix
§A.4.)
