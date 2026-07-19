# Spec §3 — Grammar

The full syntactic grammar, in the notation of §1.3. Token classes
(`IDENT`, `NUMBER`, `STRING`, …) are defined in §2. The start symbol is
`module`.

## 3.1 Modules and statements

```text
module    = { statement } ;

statement = derived-item
          | service-item
          | macro-attributed-item
          | macro-fun
          | macro-block [ ";" ]
          | macro-invocation [ ";" ]
          | "export" statement
          | expression ";"
          | if-expr        (* not before "}" — see below *)
          | for-expr       (* not before "}" *)
          | match-expr     (* not before "}" *)
          | function
          | struct
          | enum
          | impl
          | trait
          | "mod" IDENT "{" { statement } "}"
          | import ";"
          | use ";"
          | block          (* not before "}" *)
          ;
```

A block-like form (`if`/`for`/`match`/`{…}`) in statement position must
not be the last thing in its enclosing block: in that position it is
instead the block's **trailing expression** and supplies the block's value
(§3.5).

## 3.2 Imports and exports

```text
import  = "import" path-branch ;
use     = "use"    path-branch ;
path-branch = NAME [ "::" ( path-branch | path-set ) ] ;
path-set    = "{" path-branch { "," path-branch } [ "," ] "}" ;
NAME        = IDENT | "true" | "false" ;   (* variant re-exports *)
```

`import` brings names from another module into scope; `use` brings names
from a type's namespace (e.g. variants) into scope. In a set, `self` names
the item itself (`Option::{ self, Some, None }` imports the type and its
variants). Semantics: §4. `export statement` re-exports an import or
exposes a declaration to importers of the module.

## 3.3 Items

### Functions

```text
function = [ extern-attr ] [ "[" "must_use" "]" ] [ "[" "rpc" "]" ]
           [ "[" "trait_only" "]" ] [ "[" "doc" "(" "hidden" ")" "]" ]
           [ "async" ] [ "external" ]
           "fun" IDENT [ generic-params ]
           "(" [ parameter { "," parameter } [ "," ] ] ")"
           [ ":" type ] [ "borrows" IDENT ]
           ( block | ";" ) ;

parameter  = [ convention ] binder [ ":" type ] ;
convention = "own" | "&" [ "mut" ] ;
binder     = IDENT | "(" binder "," binder { "," binder } [ "," ] ")" ;

extern-attr = "[" "extern" "(" extern-args ")" "]" ;
extern-args = STRING [ "," STRING ]           (* module/global binding *)
            | ("method"|"get"|"set") [ "," STRING ] ;
```

A `;` body is a signature-only declaration — legal for `external`
functions and required trait methods. A parameter's **convention** may
come from the prefix (`own x`, `&mut self`) or from a view type
(`x: &mut T`); the prefix wins if both are present (§6.3). The `borrows`
clause names the parameter the returned view projects (§6.5).

### Structs and enums

```text
struct = [ "resource" ] [ "external" ] "struct" (IDENT | "null") [ generic-params ]
         ( "{" [ field { "," field } [ "," ] ] "}" | ";" ) ;
field  = [ "[" "expose" "]" ] IDENT [ ":" type ] ;

enum    = [ "resource" ] "enum" IDENT [ generic-params ]
          "{" [ variant { "," variant } [ "," ] ] "}" ;
variant = NAME [ "(" [ type { "," type } [ "," ] ] ")" ]
          [ "=" [ "-" ] NUMBER ] ;
```

A `;`-bodied struct is legal only for `external` structs (host types). An
explicit variant discriminant (`= 0`, `= -1`) fixes the variant's integer
tag.

The leading `resource` modifier marks a type declaration as a *resource* —
the owned-resource class, whose semantics are specified in the resources
chapter (forthcoming; the modifier currently reserves the surface). It
precedes `external`, so the full modifier order is `resource external
struct`, and it is accepted only on `struct` and `enum` declarations —
`resource` before any other item is a parse error.

### Impls and traits

```text
impl  = "impl" type [ "with" type { "+" type } ] "{" { statement } "}" ;
trait = "trait" IDENT [ generic-params ] [ "with" type { "+" type } ]
        "{" { function } "}" ;
```

An impl's subject is a **type pattern**: `type X [: bounds]` binders
anywhere inside it (`impl List<type T>`, `impl Option<(type T, type U)>`,
bare `impl type T`) declare the impl's generic parameters (§5.6). `with`
lists the implemented trait(s) — an impl without `with` provides inherent
members. A trait's `with` lists supertraits.

### Generic parameters and arguments

```text
generic-params = "<" generic-param { "," generic-param } [ "," ] ">" ;
generic-param  = [ "type" ] IDENT [ ":" ( bound-list | tuple-bound ) ]
                 [ "=" type ] ;
bound-list  = type { "+" type } ;
tuple-bound = "(" [ NUMBER ] ".." [ NUMBER ] [ ":" type ] ")" ;
generic-args = "<" type { "," type } [ "," ] ">" ;
```

A tuple bound constrains a variadic tuple parameter's arity and,
optionally, each element (`T: (2..)`, `T: (..: Display)`); see §5.9.

### Attributes and macro items

```text
derived-item   = "[" "derive" "(" IDENT { "," IDENT } [ "," ] ")" "]"
                 ( struct | enum ) ;
service-item   = "[" "service" [ "(" IDENT ")" ] "]" struct ;
macro-attributed-item = "[" IDENT [ "(" [ expr-span { "," expr-span } ] ")" ] "]"
                        ( struct | enum | function ) ;
macro-fun        = "macro" function ;
macro-invocation = "macro" IDENT "(" [ expr-span { "," expr-span } ] ")" ;
macro-block      = "macro" block ;
```

A macro attribute's arguments are captured as **source spans** — the
macro receives their text, not their values (§10). The built-in
attribute names (`derive`, `service`, `extern`, `must_use`, `rpc`,
`trait_only`, `doc`, `expose`) are not available as user macro-attribute
names.

## 3.4 Bindings and assignment

```text
let        = ("let" | "mut") binder [ ":" type ] [ "=" expression ] ;
assignment = [ "*" ] place ( "=" | "+=" | "-=" | "*=" | "/=" | "%=" )
             expression ;
ret        = "ret" [ expression ] ;
jump       = "jump" IDENT ;          (* break | continue *)
```

`let` binds immutably, `mut` mutably; a tuple binder destructures
(irrefutably — names and nested tuples only). Both the type and the
initializer are syntactically optional. A **place** is a chain expression
(§3.6) denoting a location: a local, a field chain, an index, or a place
reached through a call (`a.write().count`); the optional leading `*`
assigns through a view. `jump break` / `jump continue` control the
innermost enclosing loop.

## 3.5 Blocks and control expressions

```text
block      = "{" { statement } [ expression ] "}" ;
if-expr    = "if" secondary-expr block [ "else" ( block | if-expr ) ] ;
for-expr   = "for" IDENT "in" secondary-expr block   (* iteration *)
           | "for" secondary-expr block              (* while *)
           | "for" block ;                           (* infinite *)
match-expr = "match" secondary-expr "{" { match-leg [ "," ] } "}" ;
match-leg  = pattern { "," pattern } [ "if" expression ] "=>" expression ;
```

A block's value is its trailing expression, or `void` if none. Conditions
and `match`/`for` subjects are **secondary expressions** (§3.8) — struct
initializers are excluded there, keeping `if Foo {` unambiguous. A
match leg's comma-separated patterns form an or-pattern; the optional
`if` guard applies to the whole leg; the trailing comma after a leg is
optional.

## 3.6 Chain expressions (postfix)

The tightest expression tier, `chain`:

```text
chain   = path { call-suffix | postfix } ;
path    = ( IDENT generic-args ␣"::"  (* generic static head *)
          | atom )
          { "::" IDENT } ;
call-suffix = [ generic-args ] "(" [ expression { "," expression } [ "," ] ] ")" ;
member  = NUMBER                          (* tuple index: .0 *)
        | IDENT [ call-suffix ] ;         (* field / ONE fused method call *)
postfix = "." member
        | "[" expression "]"             (* index *)
        | "!"                            (* try-assert, §5.10 *)
        | "(" [ expression { "," expression } [ "," ] ] ")"
                                          (* direct call on the chain result *)
        | "?." member ;                  (* lift link, §5.10 *)

atom    = literal | IDENT | IDENT generic-args
        | "(" expression ")" | tuple | list
        | tuple-comprehension | macro-invocation | macro-block ;
tuple   = "(" expression "," expression { "," expression } [ "," ] ")" ;
list    = "[" [ expression { "," expression } [ "," ] ] "]" ;
tuple-comprehension = "(" IDENT "in" secondary-expr "=>" expression ")" ;
```

`Name<Args>` is read as a generic path head only when `::` immediately
follows (`List<str>::new()`); otherwise `<` is a comparison. A member
fuses at most ONE call; a further `(args)` is a **direct call** on the
chain's result, calling a closure-typed value
(`self.hook.read()(a, b)`). A `?.` link's **continuation** extends
through the following plain postfixes up to the next `?.` or `!` —
`a?.b.c()!` lifts `b.c()` into the container, then try-asserts the
result (§5.10).

## 3.7 Operator precedence

From tightest to loosest; every binary level is left-associative:

| Level | Operators | Notes |
|---|---|---|
| 1 | `::` paths, calls, `.` `[]` `!` `?.` | §3.6 |
| 2 | prefix `!` `-` `await` `async` `&` `&mut` `*` | unary; `async` also takes a block |
| 3 | `*` `/` `%` | |
| 4 | `+` `-` | |
| 5 | `<<` `>>` | the two control tokens must be span-adjacent |
| 6 | `&` | bitwise and |
| 7 | `^` | bitwise xor |
| 8 | `\|` | bitwise or |
| 9 | `==` `!=` `<` `<=` `>` `>=` | one level; `a < b < c` parses as `(a < b) < c` (ill-typed, §5.7) |
| 10 | `is` pattern | at most one per operand (no chaining) |
| 11 | `&&` | |
| 12 | `\|\|` | |

Bitwise operators bind tighter than comparisons (`a & b == c` is
`(a & b) == c`).

## 3.8 The expression tiers

```text
expression     = "const" expression        (* weak prefix: captures to the end *)
               | secondary-expr ;
secondary-expr = closure | block | if-expr | for-expr | match-expr
               | jump | let | ret | assignment
               | operator-expr ;           (* §3.7 levels 1–12 *)
condition-expr = secondary-expr ;          (* struct-init excluded from operands *)

struct-init   = IDENT [ generic-args ]
                "{" [ init-field { "," init-field } [ "," ] ] "}" ;
init-field    = IDENT [ "=" expression ] ;   (* shorthand: name alone *)
closure       = ( "||" | "|" [ closure-param { "," closure-param } [ "," ] ] "|" )
                [ ":" type ] expression ;
closure-param = binder [ ":" type ] ;
```

Two consequences of the tier split are normative:

- A **struct initializer** is an operand of the operator/postfix chain
  (`Point { … } == q` compares; `Point { x = 1, y = 2 }.length()` folds
  the member chain) — except in **condition positions**: an `if`/`for`
  condition, a `for … in` iterable, and a `match` subject parse
  `condition-expr`, whose operands exclude struct initializers, so the
  `{` after `if Foo` is the block. Parenthesize a literal to use it in a
  condition (`if p == (Point { x = 1 }) { … }`).
- `const` captures **weakly**: everything to the end of the expression
  (up to the enclosing bracket or comma) folds; parenthesize to narrow
  (§9).

A closure's body is one expression (commonly a block). `||` in operand
position always begins a zero-parameter closure; logical-or is only
recognized between two operands.

## 3.9 Types

```text
type = "&" [ "mut" ] type                       (* view type *)
     | "type" IDENT [ ":" bound-list ]          (* impl-subject binder *)
     | [ "async" | "sync" ] closure-type [ context-clause ]
     | IDENT generic-args                        (* nominal, generic *)
     | IDENT                                     (* nominal *)
     | "(" IDENT "in" type ":" type ")"          (* mapped tuple, §5.9 *)
     | "(" [ type { "," type } [ "," ] ] ")"     (* tuple type *)
     ;
closure-type   = ( "||" | "|" [ [IDENT ":"] type { "," [IDENT ":"] type } "|" )
                 [ type ] ;
context-clause = "context" ( IDENT | "(" IDENT { "," IDENT } [ "," ] ")" ) ;
```

`context` here is the contextual keyword (§2.2); the clause is only valid
on closure types, checked semantically (§8.5). `sync` is likewise
contextual (§7.4: the synchronous contract; parameters only). A closure
type's parameters may carry documentation names (`|value: T| U`); only
the types are significant.

## 3.10 Patterns (match)

```text
pattern = ("let" | "mut") binder                (* binding *)
        | "(" pattern "," pattern { "," pattern } [ "," ] ")"
        | STRING | MULTILINE_STRING | NUMBER    (* equality literal *)
        | "_"                                   (* wildcard *)
        | NAME { "::" IDENT }
          [ "(" [ pattern { "," pattern } [ "," ] ] ")" ] ;  (* variant *)
```

Bindings inside patterns are written explicitly (`Some(let x)`), so a
bare name is always a **variant** reference, never a fresh binding — the
classic mistyped-variant trap is a resolution error instead of a silent
catch-all. `bool` and `null` literals match as variants of their enums.
The `let`/parameter binder grammar (names and tuples, §3.3) is the
irrefutable subset; refutable forms (literals, variants) are match-only.
