use crate::analyzer::{
    Expr, ExprIfBranch, ExprPattern, Function, GenericDispatch, Intrinsic, LiftDispatch, Program,
    TryDispatch,
};
use crate::error::Error;
use crate::id::Id;
use crate::interpreter::ConstValue;
use crate::node::{BinaryOp, Convention, ExternBinding};
use crate::options::BuildOptions;
use crate::type_::{Type, TypeId};
use chumsky::span::Span;
use indexmap::IndexMap;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub fn transform<'src>(program: &Program<'src>, options: &BuildOptions) -> Result<String, Error> {
    Transformer::new(program, options).transform_entry()
}

/// The transformed program one step before formatting: the whole JS AST plus
/// the text prelude it needs. `transform` formats this into the final source;
/// the macro engine's interpreter (`interpreter.rs`, macro-engine.md §5)
/// evaluates `nodes` directly — the two consumers share every lowering
/// decision down to this tree.
pub struct JsProgram<'src> {
    /// Host import lines (`import { a } from "m";`) from `[extern]` bindings.
    /// Non-empty means the program reaches host capabilities — the interpreter
    /// rejects it ("not available at expansion time").
    pub imports: Vec<String>,
    /// The names of the `__` runtime helpers the program uses (`__clone`, …),
    /// in emission order. The formatter prepends their JS sources; the
    /// interpreter implements them natively by name.
    pub helpers: Vec<&'static str>,
    pub nodes: Vec<js::Node<'src>>,
}

pub fn transform_to_ast<'src>(
    program: &'src Program<'src>,
    options: &BuildOptions,
) -> Result<JsProgram<'src>, Error> {
    Transformer::new(program, options).transform_entry_ast()
}

/// Transforms a program rooted at the given FUNCTIONS instead of `main` — the
/// macro world's shape (macro-engine.md §3): macro funs are entry points the
/// expansion interpreter calls directly, so emission is seeded from them (plus
/// module-level globals) and no `main` is required. Returns the program AST and
/// each root's emitted function name. Skips the cosmetic scope-renaming pass —
/// the interpreter doesn't read the output, and stable names keep the root map
/// trivially correct.
pub fn transform_functions<'src>(
    program: &'src Program<'src>,
    options: &BuildOptions,
    roots: &[Id],
) -> Result<(JsProgram<'src>, HashMap<Id, String>), Error> {
    let mut transformer = Transformer::new(program, options);

    let global_variables = program.module_level_bindings();
    let t_global_variables = transformer.walk_list(&global_variables);

    let mut names = HashMap::new();
    for root in roots {
        transformer.ensure_function_emitted(*root);
        names.insert(*root, transformer.ng.name_for(*root));
    }

    let mut t_functions = transformer
        .required_functions
        .into_iter()
        .collect::<Vec<_>>();
    t_functions.sort_by(|a, b| (a.0.0).cmp(&b.0.0));
    let t_functions = t_functions.into_iter().map(|x| x.1);
    let t_instances = transformer.monomorphized.into_iter();

    let imports = transformer
        .used_imports
        .iter()
        .map(|(module, symbols)| {
            let names = symbols.iter().cloned().collect::<Vec<_>>().join(", ");
            format!("import {{ {} }} from \"{}\";", names, module)
        })
        .collect::<Vec<_>>();
    if !program.clone_sites.is_empty() {
        transformer.used_helpers.insert("__clone");
    }
    let helpers = transformer.used_helpers.into_iter().collect::<Vec<_>>();

    let nodes = t_functions
        .chain(t_instances)
        .chain(t_global_variables)
        .collect::<Vec<_>>();
    Ok((
        JsProgram {
            imports,
            helpers,
            nodes,
        },
        names,
    ))
}

/// Interprets a string literal's backslash escapes into the characters they
/// denote (`\n` -> newline, `\t`, `\r`, `\"`, `\\`, `\0`), so the value is the
/// real text — the JS formatter then re-escapes it for output. Borrows the slice
/// unchanged when it has no escapes. An unknown escape keeps both characters.
fn unescape_string(raw: &str) -> Cow<'_, str> {
    if !raw.contains('\\') {
        return Cow::Borrowed(raw);
    }
    let mut result = String::with_capacity(raw.len());
    let mut characters = raw.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        match characters.next() {
            Some('n') => result.push('\n'),
            Some('t') => result.push('\t'),
            Some('r') => result.push('\r'),
            Some('"') => result.push('"'),
            Some('\\') => result.push('\\'),
            Some('0') => result.push('\0'),
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => result.push('\\'),
        }
    }
    Cow::Owned(result)
}

/// The `__`-named free externs whose implementations live in the helper table:
/// a std module binds `[extern("__name")]`, and transforming a call through
/// one marks its helper for emission. Returns the canonical `'static` name.
fn extern_helper(symbol: &str) -> Option<&'static str> {
    const EXTERN_HELPERS: &[&str] = &[
        "__hmac_sha512",
        "__pbkdf2_sha512",
        "__random_bytes",
        "__db_run",
        "__db_all",
        "__db_get",
        "__db_column",
        "__db_is_null",
        "__local_get",
        "__session_get",
        "__router_path",
    ];
    EXTERN_HELPERS.iter().find(|name| **name == symbol).copied()
}

/// The JS source for a runtime helper an intrinsic call needs. `__scan` reads
/// all of stdin once and hands out one line per call; `__parse_i32` returns the
/// `Option<i32>` array form (`[0, n]` = `Some`, `[1]` = `None`).
fn helper_source(name: &str) -> &'static str {
    match name {
        "__scan" => {
            "let __vilan_stdin = null, __vilan_stdin_index = 0;\n\
             function __scan() {\n\
             \tif (__vilan_stdin === null) {\n\
             \t\ttry {\n\
             \t\t\t__vilan_stdin = require(\"fs\").readFileSync(0, \"utf-8\").split(\"\\n\");\n\
             \t\t} catch (error) {\n\
             \t\t\t__vilan_stdin = [];\n\
             \t\t}\n\
             \t}\n\
             \treturn __vilan_stdin_index < __vilan_stdin.length ? __vilan_stdin[__vilan_stdin_index++] : \"\";\n\
             }"
        }
        // STRICT parses: the whole (trimmed) text must be the number — trailing
        // garbage, a fractional part on an integer, or an out-of-range value is
        // `None`, not a truncation (`parseInt`'s liberality said the wrong thing).
        "__parse_i32" => {
            "function __parse_i32(text) {\n\
             \tconst trimmed = text.trim();\n\
             \tconst value = Number(trimmed);\n\
             \treturn /^[+-]?[0-9]+$/.test(trimmed) && value >= -2147483648 && value <= 2147483647 ? [ 0, value ] : [ 1 ];\n\
             }"
        }
        "__parse_f64" => {
            "function __parse_f64(text) {\n\
             \tconst trimmed = text.trim();\n\
             \tconst value = Number(trimmed);\n\
             \treturn trimmed === \"\" || Number.isNaN(value) ? [ 1 ] : [ 0, value ];\n\
             }"
        }
        "__try_parse_json" => {
            "function __try_parse_json(text) {\n\
             \ttry {\n\
             \t\treturn [ 0, JSON.parse(text) ];\n\
             \t} catch (error) {\n\
             \t\treturn [ 1 ];\n\
             \t}\n\
             }"
        }
        "__random_int" => {
            "function __random_int(low, high) {\n\
             \treturn Math.floor(Math.random() * (high - low + 1)) + low;\n\
             }"
        }
        "__random_float" => {
            "function __random_float(low, high) {\n\
             \treturn Math.random() * (high - low) + low;\n\
             }"
        }
        // `process::args()` — the script's own arguments: `process.argv` is
        // `[node, script, ...args]`, so the tail past index 2 is what the program
        // was invoked with. `slice` returns a fresh array (no aliasing the live
        // `argv`), matching `List` value semantics.
        "__args" => {
            "function __args() {\n\
             \treturn process.argv.slice(2);\n\
             }"
        }
        // `Shared::new(value)` — a one-field object cell. An object (not an array)
        // is returned by reference from `__clone`, so the cell is shared, not
        // snapshotted — exactly the `Shared` semantics.
        "__shared_new" => {
            "function __shared_new(value) {\n\
             \treturn { v: value };\n\
             }"
        }
        // `process::env(key): Option<str>` — a missing variable reads back
        // `undefined`, which becomes `None`; otherwise `Some(value)`.
        "__env" => {
            "function __env(key) {\n\
             \tconst value = process.env[key];\n\
             \treturn value === undefined ? [ 1 ] : [ 0, value ];\n\
             }"
        }
        // `List.get(i): Option<T>` — bounds-checked, returning the `Option` array
        // form. Clones the element so the returned value can't alias the list
        // (value semantics; views are second-class and can't escape).
        "__list_get" => {
            "function __list_get(list, index) {\n\treturn index >= 0 && index < list.length ? [ 0, __clone(list[index]) ] : [ 1 ];\n}"
        }
        // WebCrypto glue (std::crypto): HMAC-SHA-512 over `crypto.subtle`.
        "__hmac_sha512" => {
            "async function __hmac_sha512(key, data) {\n\
             \tconst imported = await crypto.subtle.importKey(\"raw\", key, { name: \"HMAC\", hash: \"SHA-512\" }, false, [ \"sign\" ]);\n\
             \treturn new Uint8Array(await crypto.subtle.sign(\"HMAC\", imported, data));\n\
             }"
        }
        // PBKDF2-HMAC-SHA-512 via `crypto.subtle.deriveBits`.
        "__pbkdf2_sha512" => {
            "async function __pbkdf2_sha512(password, salt, iterations, bits) {\n\
             \tconst imported = await crypto.subtle.importKey(\"raw\", password, \"PBKDF2\", false, [ \"deriveBits\" ]);\n\
             \treturn new Uint8Array(await crypto.subtle.deriveBits({ name: \"PBKDF2\", salt, iterations, hash: \"SHA-512\" }, imported, bits));\n\
             }"
        }
        // Web Storage glue (std::storage): a missing key reads null; flatten to "".
        "__local_get" => {
            "function __local_get(key) {\n\treturn localStorage.getItem(key) ?? \"\";\n}"
        }
        "__session_get" => {
            "function __session_get(key) {\n\treturn sessionStorage.getItem(key) ?? \"\";\n}"
        }
        // Router glue (std::router): `location.pathname` is a global property,
        // which the function-extern form can't address directly.
        "__router_path" => "function __router_path() {\n\treturn location.pathname;\n}",
        // SQLite glue (std::db): parameter spreads and row/column reads the
        // extern binding forms can't express directly.
        "__db_run" => {
            "function __db_run(statement, parameters) {\n\
             \tconst result = statement.run(...parameters);\n\
             \treturn Number(result.lastInsertRowid ?? 0);\n\
             }"
        }
        "__db_all" => {
            "function __db_all(statement, parameters) {\n\treturn statement.all(...parameters);\n}"
        }
        "__db_get" => {
            "function __db_get(statement, parameters) {\n\
             \tconst row = statement.get(...parameters);\n\
             \treturn row === undefined ? [ 1 ] : [ 0, row ];\n\
             }"
        }
        "__db_column" => "function __db_column(row, name) {\n\treturn row[name];\n}",
        "__db_is_null" => {
            "function __db_is_null(row, name) {\n\treturn row[name] === null || row[name] === undefined;\n}"
        }
        // Cryptographically random bytes.
        "__random_bytes" => {
            "function __random_bytes(length) {\n\treturn crypto.getRandomValues(new Uint8Array(length));\n}"
        }
        // `list[i]` — the checked subscript read: out of bounds panics (`get`
        // is the total, Option-returning form above).
        "__at" => {
            "function __at(list, index) {\n\
             \tif (index >= 0 && index < list.length) return list[index];\n\
             \tthrow \"index out of bounds: the length is \" + list.length + \" but the index is \" + index;\n\
             }"
        }
        // `list[i] = v` — the checked subscript write: writing never creates a
        // slot (growth is `push`), so out of bounds panics.
        "__at_put" => {
            "function __at_put(list, index, value) {\n\
             \tif (index >= 0 && index < list.length) return list[index] = value;\n\
             \tthrow \"index out of bounds: the length is \" + list.length + \" but the index is \" + index;\n\
             }"
        }
        // `&mut list[i]` — the checked view mint: the scalar `(base, key)` pair
        // exists only for an in-bounds element.
        "__at_view" => {
            "function __at_view(list, index) {\n\
             \tif (index >= 0 && index < list.length) return [ list, index ];\n\
             \tthrow \"index out of bounds: the length is \" + list.length + \" but the index is \" + index;\n\
             }"
        }
        // `List.pop(): Option<T>` — removes and returns the last element (no clone:
        // the element leaves the list), or `None` when empty.
        "__list_pop" => {
            "function __list_pop(list) {\n\treturn list.length === 0 ? [ 1 ] : [ 0, list.pop() ];\n}"
        }
        // `Map.get(key): Option<V>` — returns the `Option` array form, cloning the
        // value so the result can't alias the map (value semantics).
        "__map_get" => {
            "function __map_get(map, key) {\n\treturn map.has(key) ? [ 0, __clone(map.get(key)) ] : [ 1 ];\n}"
        }
        // `Map.keys()`/`Map.values(): List<_>` — a fresh array snapshot (cloned, so
        // it can't alias the map's stored entries) in insertion order.
        "__map_keys" => "function __map_keys(map) {\n\treturn [ ...map.keys() ].map(__clone);\n}",
        "__map_values" => {
            "function __map_values(map) {\n\treturn [ ...map.values() ].map(__clone);\n}"
        }
        // `for x in set`: `Set` is a struct `[table]` over a `NativeMap`, so the
        // elements are the backing map's stored originals, in insertion order (I1).
        "__set_iter" => "function __set_iter(set) {\n\treturn [ ...set[0].values() ];\n}",
        // The externally-tagged enum discriminator: a bare `"Variant"` is its own
        // tag, a `{"Variant":..}` object's tag is its single key.
        "__json_tag" => {
            "function __json_tag(value) {\n\treturn typeof value === \"string\" ? value : Object.keys(value)[0];\n}"
        }
        // The normalized JSON type of a parsed value: `typeof` buckets arrays and
        // `null` as `"object"`, so name them explicitly. Basis for the decode
        // type checks (`JsonValue.kind()` in json.vl).
        "__json_kind" => {
            "function __json_kind(value) {\n\tif (value === null) return \"null\";\n\tif (Array.isArray(value)) return \"array\";\n\treturn typeof value;\n}"
        }
        // The canonical key of a value: a primitive keys as itself (JS keys those
        // by value), an aggregate (an object/array) canonicalizes to its JSON
        // string. Basis of `Hashable` / value-keyed `Map`/`Set`.
        "__hash" => {
            "function __hash(value) {\n\treturn (typeof value === \"object\" && value !== null) ? JSON.stringify(value) : value;\n}"
        }
        // Value-semantics deep clone. Structs/lists/enums/tuples are arrays and a
        // `Set`/`Map` is a JS `Set`/`Map`, so recurse into them; everything else —
        // primitives and closures — is returned by reference (a closure is
        // immutable, so sharing it is a copy). Unlike `structuredClone`, this
        // doesn't throw on functions.
        "__clone" => {
            "function __clone(value) {\n\
             \tif (Array.isArray(value)) return value.map(__clone);\n\
             \tif (value instanceof Set) return new Set([ ...value ].map(__clone));\n\
             \tif (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));\n\
             \treturn value;\n\
             }"
        }
        _ => "",
    }
}

/// Whether two types name the same nominal struct/enum, ignoring type
/// arguments — so an `impl List<T>` (subject `List<Generic>`) matches a concrete
/// `List<i32>` value when resolving a member to emit.
fn nominal_matches(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Struct(a_id, _), Type::Struct(b_id, _)) => a_id == b_id,
        (Type::Enum(a_id, _), Type::Enum(b_id, _)) => a_id == b_id,
        _ => a == b,
    }
}

/// Builds a binary expression, gluing adjacent string literals at compile time
/// so concatenations like `"" + "Hello, " + "!"` collapse to a single literal.
/// Because `+` is left-associative, folding here folds whole static runs.
fn binary<'src>(op: BinaryOp, lhs: js::Node<'src>, rhs: js::Node<'src>) -> js::Node<'src> {
    match (op, lhs, rhs) {
        (BinaryOp::Add, js::Node::String(left), js::Node::String(right)) => {
            let mut glued = left.into_owned();
            glued.push_str(&right);
            js::Node::String(Cow::Owned(glued))
        }
        (op, lhs, rhs) => js::Node::Binary(op, Box::new(lhs), Box::new(rhs)),
    }
}

/// How a dispatched trait-member call lowers, once resolved to a concrete type's
/// member. The member may be an intrinsic or an `[extern]` external (a host form),
/// not just a normal emitted function — so resolution is split from emission, and
/// `args` is consumed only once the form is known (see `resolve_dispatch`).
enum Dispatch<'src> {
    /// A built-in lowering (`str.len()` → `.length`, etc.).
    Intrinsic(Intrinsic),
    /// An `[extern]`-bound external: the external's id and its host binding.
    Extern(Id, ExternBinding<'src>),
    /// A normal emitted function: its JS name and whether it is async.
    Call(String, bool),
}

struct Transformer<'src> {
    formatter: Formatter,
    ng: NameGenerator,
    print_fn_id: Id,
    list_new_fn_id: Option<Id>,
    list_push_fn_id: Option<Id>,
    panic_fn_id: Option<Id>,
    program: &'src Program<'src>,
    required_functions: IndexMap<Id, js::Node<'src>>,
    // Functions whose body is currently being walked. A recursive (or mutually
    // recursive) call inside that body must not re-enter and re-emit it — the
    // call site only needs the function's name, which is available regardless.
    // Kept separate from `required_functions` (which records *finished* bodies)
    // so the callee-before-caller insertion order is preserved.
    emitting: HashSet<Id>,
    // The active generic-parameter substitution while emitting a monomorphized
    // function body (constraint id -> concrete type id).
    current_substitution: HashMap<TypeId, TypeId>,
    // Every entity emitted as a VALUE reference (the `Expr::Local` arm) —
    // consulted at assembly to tree-shake module-level bindings (F6): a
    // binding emits only if something reachable referenced it.
    referenced_globals: HashSet<Id>,
    // Monomorphized function variants, keyed by (generic function, concrete
    // type arguments) so each distinct instantiation is emitted exactly once.
    instances: HashMap<(Id, Vec<String>), String>,
    // The concrete type a trait default method is currently being specialized
    // for, so `self.method()` calls in its body re-dispatch to that type's impl.
    current_self_type: Option<TypeId>,
    // Trait default methods specialized per concrete type, keyed by
    // (default function, concrete type) so each is emitted once.
    default_instances: HashMap<(Id, String), String>,
    monomorphized: Vec<js::Node<'src>>,
    // Captures introduced by an `is` test, aliased to the subject's payload
    // slots (e.g. `t[1]`) since they can't be JS bindings in expression position.
    is_bindings: HashMap<Id, js::Node<'src>>,
    // Runtime helper functions (`__scan`, `__parse_i32`, `__random_int`) an
    // intrinsic call needs; emitted as a prelude only when used.
    used_helpers: BTreeSet<&'static str>,
    // Host imports an `[extern]` call needs, as module -> imported symbols;
    // emitted as `import { a, b } from "module";` lines at the top.
    used_imports: BTreeMap<String, BTreeSet<String>>,
}

impl<'src> Transformer<'src> {
    fn new(program: &'src Program<'src>, options: &BuildOptions) -> Self {
        let style = if options.readable_names {
            NameStyle::Readable
        } else if options.debug_names {
            NameStyle::Annotated
        } else {
            NameStyle::Plain
        };
        // Source names for functions, variables, and parameters — what `Readable`
        // names identifiers after and `Annotated` annotates them with. `Plain`
        // needs none.
        let source_names = if matches!(style, NameStyle::Plain) {
            HashMap::new()
        } else {
            program
                .variables
                .iter()
                .map(|(id, variable)| (*id, variable.name.to_string()))
                .chain(
                    program
                        .functions
                        .iter()
                        .map(|(id, function)| (*id, function.name.to_string())),
                )
                .chain(
                    program
                        .parameters
                        .iter()
                        .map(|(id, parameter)| (*id, parameter.name.to_string())),
                )
                .collect::<HashMap<Id, String>>()
        };
        let reserved = if options.readable_names {
            collect_reserved_names(program)
        } else {
            HashSet::new()
        };

        let print_fn_id = {
            let std_module_id = *program
                .module_id_by_name
                .get("std")
                .expect("missing std module");
            let std_module = program.modules.get(&std_module_id).unwrap();
            let std_module_scope_id = std_module.body.1;
            let std_module_scope = program.scopes.get(&std_module_scope_id).unwrap();
            let print_fn_id = *std_module_scope
                .name_to_id_map
                .get("print")
                .expect("missing print function in the std module");
            print_fn_id
        };

        Self {
            formatter: Formatter::from_options(options.indent, options.spaces),
            ng: NameGenerator::new(style, source_names, reserved),
            print_fn_id,
            list_new_fn_id: program.list_new_fn_id,
            list_push_fn_id: program.list_push_fn_id,
            panic_fn_id: program.panic_fn_id,
            program,
            required_functions: IndexMap::new(),
            emitting: HashSet::new(),
            current_substitution: HashMap::new(),
            referenced_globals: HashSet::new(),
            instances: HashMap::new(),
            current_self_type: None,
            default_instances: HashMap::new(),
            monomorphized: Vec::new(),
            is_bindings: HashMap::new(),
            used_helpers: BTreeSet::new(),
            used_imports: BTreeMap::new(),
        }
    }

    fn transform_entry(self) -> Result<String, Error> {
        let formatter = self.formatter.clone();
        let line_break = formatter.line_break;
        let program = self.transform_entry_ast()?;
        let body = formatter.file(&program.nodes);
        let imports = program.imports.join("\n");
        let helpers = program
            .helpers
            .iter()
            .map(|name| helper_source(name))
            .collect::<Vec<_>>()
            .join("\n");
        let prelude = [imports, helpers]
            .into_iter()
            .filter(|section| !section.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let output = if prelude.is_empty() {
            body
        } else {
            format!("{}\n{}", prelude, body)
        };
        Ok(format!("{}{}", output, line_break))
    }

    fn transform_entry_ast(mut self) -> Result<JsProgram<'src>, Error> {
        let global_scope = self
            .program
            .scopes
            .get(&self.program.global_scope_id)
            .unwrap();

        // Every module-level binding, in declaration order — the entry's own
        // globals plus loaded modules' top-level `let`s (one shared
        // definition; platform coloring admits the same set it emits).
        let global_variables = self.program.module_level_bindings();

        let main_fn = global_scope
            .name_to_id_map
            .get("main")
            .and_then(|id| self.program.functions.get(id))
            .ok_or_else(|| Error {
                msg: "Cannot execute program without a main function".to_string(),
                span: Span::new((), 0..0),
            })?;
        let main_is_async = self.program.async_functions.contains(&main_fn.id);

        // Walk the module-level bindings the entry can REACH, in declaration
        // order, keeping each binding's nodes separate (F6 — a binding emits
        // only if something reachable references it; the stated semantics: a
        // dropped binding's initializer does not run — module state exists
        // only if something reaches it; top-level side effects are not a
        // promise). Reachability comes from the call graph — the same edges
        // platform coloring admits over — so a dropped binding is never even
        // walked: its initializer can't retain callees, nor drag their host
        // `import ... from "node:..."` lines into a bundle that never runs
        // it. Assembly still keeps only bindings that emitted code actually
        // referenced (dispatch candidates over-approximate reachability, like
        // everywhere else — such a binding is walked but then dropped here,
        // and it was admission-checked by the same graph).
        let graph = crate::call_graph::CallGraph::build(self.program);
        let reachable_bindings =
            crate::platform_color::reachable_bindings(self.program, &graph, main_fn.id);
        let binding_nodes: Vec<(Id, Vec<js::Node<'src>>)> = global_variables
            .iter()
            .filter(|binding| reachable_bindings.contains(binding))
            .map(|&binding| (binding, self.walk_list(&vec![binding])))
            .collect();

        let mut t_main_fn_body = self.walk_list(&main_fn.body.0);

        // Emit main's trailing expression (and any statements it expands to). On
        // Node a non-void result is forwarded to `process.exit` (the exit code); a
        // void tail (e.g. a block ending in a loop) exits normally. The browser has
        // no exit code, so the tail is emitted as a plain statement — its side
        // effects still run (a `main` that ends in `render()`), the value discarded.
        if let Some(value) = self.walk_entity(main_fn.body.1, &mut t_main_fn_body) {
            if !matches!(value, js::Node::Void) {
                // A host with `process.exit` (Node) forwards `main`'s result as the
                // exit code; the browser (and the host-less `none`, which the CLI
                // refuses to *build*) has none, so the tail is a plain statement.
                let statement = if self.program.platform.has_process_exit() {
                    js::Node::Call(
                        Box::new(js::Node::Property(
                            Box::new(js::Node::Local("process".to_string())),
                            "exit".to_string(),
                        )),
                        vec![value],
                    )
                } else {
                    value
                };
                t_main_fn_body.push(statement);
            }
        }

        // An async `main` (it awaits) runs inside an invoked async arrow, since
        // top-level `await` isn't assumed: `(async () => { .. })()`.
        if main_is_async {
            t_main_fn_body = vec![js::Node::Call(
                Box::new(js::Node::Closure(js::Closure {
                    parameters: Vec::new(),
                    body: t_main_fn_body,
                    is_async: true,
                })),
                Vec::new(),
            )];
        }

        // Assembly-time tree-shake: keep a binding's declaration only when
        // something emitted referenced it.
        let t_global_variables: Vec<js::Node<'src>> = binding_nodes
            .into_iter()
            .filter(|(binding, _)| self.referenced_globals.contains(binding))
            .flat_map(|(_, nodes)| nodes)
            .collect();

        let mut t_functions = self.required_functions.into_iter().collect::<Vec<_>>();
        t_functions.sort_by(|a, b| (a.0.0).cmp(&b.0.0));
        let t_functions = t_functions.into_iter().map(|x| x.1);

        // Monomorphized variants are plain function declarations too; ordering
        // among declarations is irrelevant since JS hoists them.
        let t_instances = self.monomorphized.into_iter();

        // Host imports (`import { a, b } from "module";`) from `[extern]` calls,
        // then runtime helpers (`__scan`, ...) — both a prelude before the body.
        let imports = self
            .used_imports
            .iter()
            .map(|(module, symbols)| {
                let names = symbols.iter().cloned().collect::<Vec<_>>().join(", ");
                format!("import {{ {} }} from \"{}\";", names, module)
            })
            .collect::<Vec<_>>();
        // Value-semantics copies (`own` arguments, aggregate bindings) lower to
        // the `__clone` helper rather than `structuredClone`, which can't copy
        // the closures a struct may hold.
        if !self.program.clone_sites.is_empty() {
            self.used_helpers.insert("__clone");
        }
        let helpers = self.used_helpers.into_iter().collect::<Vec<_>>();

        let mut nodes = t_functions
            .chain(t_instances)
            .chain(t_global_variables)
            .chain(t_main_fn_body)
            .collect::<Vec<_>>();
        // Re-allocate names over the JS scope tree so disjoint scopes share them
        // (readable: both sibling `value`s stay `value`; release: reuse short
        // names per function).
        rename_for_scopes(&self.ng, self.program, &mut nodes);
        Ok(JsProgram {
            imports,
            helpers,
            nodes,
        })
    }

    /// Push an expression-position result into `body`: normally an
    /// assignment into `result_name`, but a DIVERGING value (`return`,
    /// `break`, `continue` — a `Never`-typed match leg or if branch) is a
    /// statement of its own; `x = return e` is not JavaScript.
    fn push_result_or_divergence(
        &mut self,
        result_name: &str,
        value: js::Node<'src>,
        body: &mut Vec<js::Node<'src>>,
    ) {
        match value {
            js::Node::Return(_) | js::Node::Break | js::Node::Continue => body.push(value),
            value => body.push(js::Node::Assignment(
                Box::new(js::Node::Local(result_name.to_string())),
                Box::new(value),
            )),
        }
    }

    fn walk_list(&mut self, list: &Vec<Id>) -> Vec<js::Node<'src>> {
        let mut block = Vec::new();
        self.walk_entities(list, &mut block);
        block
    }

    fn walk_entities(&mut self, ids: &Vec<Id>, mut block: &mut Vec<js::Node<'src>>) {
        for id in ids {
            if let Some(node) = self.walk_entity(*id, &mut block) {
                // A statement whose value is discarded and is `undefined` (e.g.
                // the trailing void of a block used as a statement) is a no-op.
                if matches!(node, js::Node::Void) {
                    continue;
                }
                block.push(node);
            }
        }
    }

    /// Wraps a call in `await` when its target is async (the implicit await), so
    /// the value flows as the resolved `T` rather than a promise.
    fn maybe_await(&self, target_id: Id, node: js::Node<'src>) -> js::Node<'src> {
        // `async_values`: a call through an `async || T`-typed parameter or
        // binding awaits like a direct async call (J2).
        if self.program.async_functions.contains(&target_id)
            || self.program.async_values.contains(&target_id)
        {
            js::Node::Await(Box::new(node))
        } else {
            node
        }
    }

    /// Rule 1 (value semantics): wrap a value in `__clone(...)` when the analyzer
    /// marked this binding/assignment as copying an aggregate place that would
    /// otherwise alias its source. `__clone` (not `structuredClone`) so a value
    /// holding closures can be copied.
    fn maybe_clone(&self, value_id: Id, node: js::Node<'src>) -> js::Node<'src> {
        if self.program.clone_sites.contains(&value_id) {
            js::Node::Call(Box::new(js::Node::Local("__clone".to_string())), vec![node])
        } else {
            node
        }
    }

    /// Whether an expression may have a side effect — a call, an `await`, or an
    /// assignment, or anything containing one. An unused `let` binding can be
    /// dropped only if its initializer is side-effect-free; a side-effecting one
    /// (e.g. a call that mutates through `&mut self`) must still run.
    fn expr_has_side_effects(&self, expr_id: Id) -> bool {
        match self.program.entity_map.get(&expr_id) {
            Some(Expr::Call(_)) | Some(Expr::Await(_)) | Some(Expr::Assignment(_, _)) => true,
            // An `async { .. }` block is an *invoked* async arrow — it starts
            // executing its body immediately, so it is effectful even when its
            // promise is discarded (`let _ = async { pump loop }`).
            Some(Expr::Async(_)) => true,
            Some(Expr::Binary(_, lhs, rhs)) => {
                self.expr_has_side_effects(*lhs) || self.expr_has_side_effects(*rhs)
            }
            Some(Expr::Unary(_, operand))
            | Some(Expr::Reference(operand, _))
            | Some(Expr::Dereference(operand)) => self.expr_has_side_effects(*operand),
            Some(Expr::Field(subject, _, _)) | Some(Expr::TupleIndex(subject, _, _)) => {
                self.expr_has_side_effects(*subject)
            }
            // A checked subscript can panic, so an indexing expression is
            // effectful in itself: dropping it would drop its bounds check.
            Some(Expr::Index(_, _)) => true,
            Some(Expr::List(ids)) | Some(Expr::Tuple(ids)) => {
                ids.iter().any(|id| self.expr_has_side_effects(*id))
            }
            Some(Expr::StructInitializer(_, fields)) => {
                fields.values().any(|id| self.expr_has_side_effects(*id))
            }
            // A comprehension runs its body per element (`combine` subscribes each
            // source this way), so it inherits the body's side effects.
            Some(Expr::TupleComprehension(_, _, body_id)) => self.expr_has_side_effects(*body_id),
            _ => false,
        }
    }

    /// Whether a deref operand is a scalar `(base, key)` view — so `*operand`
    /// reads or writes `operand[0][operand[1]]`. True for a scalar-view binding /
    /// parameter, or `&place` of a scalar place directly.
    fn derefs_scalar_view(&self, operand: Id) -> bool {
        match self.program.entity_map.get(&operand) {
            Some(Expr::Local(binding)) => {
                self.program.primitive_views.contains(binding)
                    || self.generic_ref_param_is_scalar(*binding)
            }
            Some(Expr::Reference(..)) => self.program.scalar_view_refs.contains(&operand),
            // `*obj.slot()` — a `borrows` call returning a scalar view.
            Some(Expr::Call(..)) => self.program.scalar_view_calls.contains(&operand),
            _ => false,
        }
    }

    /// A `&`/`&mut` parameter whose declared pointee is a generic that resolves,
    /// at this monomorphization, to a scalar primitive. `compute_primitive_views`
    /// classifies a view by its pointee type, but a generic `&mut T` parameter's
    /// pointee is abstract there, so it cannot be added to `primitive_views`; the
    /// classification is re-made here against the concrete type, so a scalar
    /// pointee uses the `(base, key)` representation its (concrete) caller passed
    /// rather than the aggregate `Object.assign` path.
    fn generic_ref_param_is_scalar(&self, binding: Id) -> bool {
        self.program
            .parameters
            .get(&binding)
            .is_some_and(|parameter| {
                matches!(parameter.convention, Convention::Ref | Convention::RefMut)
                    && matches!(
                        self.program.type_id_to_type_map.get(&parameter.type_id),
                        Some(Type::Generic(_))
                    )
                    && self.resolves_to_scalar_primitive(parameter.type_id)
            })
    }

    /// Whether `type_id`, resolved under the active monomorphization substitution,
    /// is one of the scalar primitives that take a `(base, key)` view (the same
    /// set as the analyzer's `is_scalar_primitive`).
    fn resolves_to_scalar_primitive(&self, type_id: TypeId) -> bool {
        matches!(
            self.program.type_id_to_type_map.get(&self.resolve_type_id(type_id)),
            Some(Type::Struct(id, _))
                if self.program.structs.get(id).is_some_and(|struct_|
                    matches!(
                    struct_.name,
                    "str" | "i32"
                        | "u32"
                        | "f64"
                        | "BigInt"
                        | "null"
                        | "i8"
                        | "u8"
                        | "i16"
                        | "u16"
                        | "i53"
                        | "u53"
                        | "f32"
                ))
        )
    }

    /// Whether a bitwise/shift binary's operands are `u32` — the emission
    /// switch between JS's signed operators and the `>>>`-based unsigned forms.
    /// A concrete-`u32` verdict was recorded by the analyzer; a generic operand
    /// recorded its constraint, resolved here under the active
    /// monomorphization's substitution.
    fn binary_operands_are_u32(&self, binary_id: Id) -> bool {
        if self.program.bitwise_u32.contains(&binary_id) {
            return true;
        }
        let Some(constraint_id) = self.program.bitwise_generic_lhs.get(&binary_id) else {
            return false;
        };
        matches!(
            self.program
                .type_id_to_type_map
                .get(&self.resolve_type_id(*constraint_id)),
            Some(Type::Struct(id, _))
                if self.program.structs.get(id).is_some_and(|struct_| struct_.name == "u32")
        )
    }

    /// Whether a division's operands are an INTEGER primitive — the switch to
    /// the truncating `Math.trunc` emission (proposal/numeric-types.md §2).
    /// Concrete verdicts were recorded by the analyzer; a generic operand
    /// resolves under the active monomorphization's substitution.
    fn binary_operands_are_integer(&self, binary_id: Id) -> bool {
        const INTEGER_PRIMITIVES: &[&str] = &["i8", "u8", "i16", "u16", "i32", "u32", "i53", "u53"];
        if self.program.integer_division.contains(&binary_id) {
            return true;
        }
        let Some(constraint_id) = self.program.division_generic_lhs.get(&binary_id) else {
            return false;
        };
        matches!(
            self.program
                .type_id_to_type_map
                .get(&self.resolve_type_id(*constraint_id)),
            Some(Type::Struct(id, _))
                if self
                    .program
                    .structs
                    .get(id)
                    .is_some_and(|struct_| INTEGER_PRIMITIVES.contains(&struct_.name))
        )
    }

    /// Whether a local is boxed into a `[value]` cell at this monomorphization: a
    /// concrete scalar root (`boxed_locals`), or a generic-typed `&`-referenced
    /// root that resolves here to a scalar primitive (decided now, not in the
    /// analyzer, since its type was abstract there).
    fn local_is_boxed(&self, id: Id) -> bool {
        self.program.boxed_locals.contains(&id)
            || (self.program.generic_referenced_roots.contains(&id)
                && self
                    .program
                    .variables
                    .get(&id)
                    .is_some_and(|variable| self.resolves_to_scalar_primitive(variable.type_id)))
    }

    /// Whether `&[mut] operand` (the reference expr `ref_id`) lowers to a scalar
    /// `(base, key)` pair: a concrete scalar place (`scalar_view_refs`), or a
    /// reference whose place root is a generic local resolving here to a scalar.
    fn emits_scalar_view_ref(&self, ref_id: Id, operand: Id) -> bool {
        self.program.scalar_view_refs.contains(&ref_id)
            || self.place_root_local(operand).is_some_and(|root| {
                self.program.generic_referenced_roots.contains(&root)
                    && self
                        .program
                        .variables
                        .get(&root)
                        .is_some_and(|variable| self.resolves_to_scalar_primitive(variable.type_id))
            })
    }

    /// The local a place expression bottoms out in (mirrors the analyzer's
    /// `place_root`) — for deciding a generic place's view representation.
    fn place_root_local(&self, expr_id: Id) -> Option<Id> {
        match self.program.entity_map.get(&expr_id)? {
            Expr::Local(binding) => Some(*binding),
            Expr::Field(subject, _, _) | Expr::TupleIndex(subject, _, _) => {
                self.place_root_local(*subject)
            }
            Expr::Index(subject, _) => self.place_root_local(*subject),
            Expr::Dereference(operand) => self.place_root_local(*operand),
            _ => None,
        }
    }

    /// Whether an expression is a `Shared::write()` call — a single-slot view of
    /// the cell's `v` slot. Writing through it rebinds the slot (`cell.v = x`),
    /// distinct from both the `(base, key)` and aggregate-`Object.assign` views.
    fn is_shared_write(&self, operand: Id) -> bool {
        let Some(Expr::Call(call_id)) = self.program.entity_map.get(&operand) else {
            return false;
        };
        let Some(function_call) = self.program.function_calls.get(call_id) else {
            return false;
        };
        let Some(Expr::Local(function_id)) = self.program.entity_map.get(&function_call.subject_id)
        else {
            return false;
        };
        matches!(
            self.program.intrinsics.get(function_id),
            Some(Intrinsic::SharedWrite)
        )
    }

    fn walk_entity(&mut self, id: Id, block: &mut Vec<js::Node<'src>>) -> Option<js::Node<'src>> {
        // A `const` expression's computed value replaces the whole subtree —
        // in-place serialization (const-eval.md §1). The const mini-programs
        // themselves are built by `transform_const_program` with the results
        // map still empty for the expression being evaluated, so this arm
        // never short-circuits an evaluation.
        if let Some(value) = self.program.const_results.get(&id) {
            return Some(const_value_to_js(value));
        }
        let entity = self.program.entity_map.get(&id).unwrap();

        Some(match entity {
            Expr::Error => unreachable!(),
            // A macro-name marker: never a value (the analyzer rejects value
            // uses); reached only as an inert statement — emit nothing.
            Expr::Macro => js::Node::Void,
            Expr::TupleComprehension(binder_id, source_id, body_id) => {
                // A flat tuple is a JS array, so the comprehension lowers to a
                // runtime `source.map((x) => body)` — arity-independent, no
                // monomorphization needed. The binder is the closure parameter.
                let (binder_id, source_id, body_id) = (*binder_id, *source_id, *body_id);
                let source = self.walk_entity(source_id, block).unwrap_or(js::Node::Void);
                let parameter_name = self.ng.name_for(binder_id);
                let mut body = Vec::new();
                if let Some(value) = self.walk_entity(body_id, &mut body) {
                    body.push(js::Node::Return(Box::new(value)));
                }
                let closure = js::Node::Closure(js::Closure {
                    parameters: vec![js::Parameter {
                        name: parameter_name,
                    }],
                    body,
                    is_async: false,
                });
                js::Node::Call(
                    Box::new(js::Node::Property(Box::new(source), "map".to_string())),
                    vec![closure],
                )
            }
            Expr::Void => js::Node::Void,
            Expr::Null => js::Node::Null,
            Expr::Bool(x) => js::Node::Bool(*x),
            Expr::Number(whole, fraction, suffix) => {
                // `n`-suffixed literals are JS BigInts (`5n`); other suffixes
                // only affect typing and are dropped in the output.
                let whole = if matches!(*suffix, Some("n")) {
                    format!("{whole}n")
                } else {
                    whole.to_string()
                };
                js::Node::Number(whole, fraction.map(|x| x.to_string()))
            }
            Expr::String(x) => js::Node::String(unescape_string(x)),
            // A triple-quoted string: RAW (no escape interpretation), trimmed
            // to its content; the analyzer already validated, so an error here
            // is unreachable and degrades to "".
            Expr::MultilineString(x) => js::Node::String(std::borrow::Cow::Owned(
                crate::util::trim_multiline_string(x).unwrap_or_default(),
            )),
            Expr::Struct(_) => {
                return None;
            }
            Expr::Enum(_) => {
                return None;
            }
            Expr::Trait(_) => {
                return None;
            }
            Expr::Impl(_) => {
                return None;
            }
            Expr::ExternalFunction(_) => {
                return None;
            }
            Expr::Generic(_) => {
                return None;
            }
            Expr::Function(id) => {
                let function = self.program.functions.get(id).unwrap();
                self.function(function)
            }
            // An enum value is an array whose first element identifies the
            // variant; a bare (data-less) variant is just `[index]`. `bool` is
            // the exception — it lowers to a native boolean.
            Expr::EnumVariant(enum_id, variant_index) => {
                self.variant_value(*enum_id, *variant_index, Vec::new())
            }
            Expr::Local(id) => {
                self.referenced_globals.insert(*id);
                // A capture from an `is` test aliases the subject's payload slot.
                if let Some(accessor) = self.is_bindings.get(id) {
                    return Some(accessor.clone());
                }
                // A reference to a data-less variant (e.g. `None`) is the
                // variant value itself, not a named binding.
                if let Some(Expr::EnumVariant(enum_id, variant_index)) =
                    self.program.entity_map.get(id)
                {
                    return Some(self.variant_value(*enum_id, *variant_index, Vec::new()));
                }
                // A reference to a named function as a VALUE (backlog B20,
                // proposal/fn-coercion.md): the function object itself is the
                // value — ensure it's emitted and name it, exactly as a call
                // subject would.
                if let Some(Expr::Function(function_id)) = self.program.entity_map.get(id) {
                    let function_id = *function_id;
                    self.ensure_function_emitted(function_id);
                    return Some(js::Node::Local(self.ng.name_for(function_id)));
                }
                // A boxed scalar local reads through its cell's slot 0.
                if self.local_is_boxed(*id) {
                    return Some(js::Node::PropertyIndex(
                        Box::new(js::Node::Local(self.ng.name_for(*id))),
                        Box::new(js::Node::Number("0".to_string(), None)),
                    ));
                }
                js::Node::Local(self.ng.name_for(*id))
            }
            Expr::Field(subject_id, _struct_id, field_index) => {
                let subject = self
                    .walk_entity(*subject_id, block)
                    .unwrap_or(js::Node::Void);
                js::Node::PropertyIndex(
                    Box::new(subject),
                    Box::new(js::Node::Number(field_index.to_string(), None)),
                )
            }
            // `pair.0` — tuples store flat: a width-1 element reads its slot,
            // a tuple-typed element reslices its region (like destructuring).
            Expr::TupleIndex(subject_id, offset, width) => {
                let subject = self
                    .walk_entity(*subject_id, block)
                    .unwrap_or(js::Node::Void);
                if *width == 1 {
                    js::Node::PropertyIndex(
                        Box::new(subject),
                        Box::new(js::Node::Number(offset.to_string(), None)),
                    )
                } else {
                    js::Node::Call(
                        Box::new(js::Node::Property(Box::new(subject), "slice".to_string())),
                        vec![
                            js::Node::Number(offset.to_string(), None),
                            js::Node::Number((offset + width).to_string(), None),
                        ],
                    )
                }
            }
            // `list[i]` — the checked read (`__at`): an out-of-bounds subscript
            // panics; `get` is the total, Option-returning form.
            Expr::Index(subject_id, index_id) => {
                let subject = self
                    .walk_entity(*subject_id, block)
                    .unwrap_or(js::Node::Void);
                let index = self.walk_entity(*index_id, block).unwrap_or(js::Node::Void);
                self.used_helpers.insert("__at");
                js::Node::Call(
                    Box::new(js::Node::Local("__at".to_string())),
                    vec![subject, index],
                )
            }
            Expr::Call(id) => {
                let function_call = self.program.function_calls.get(id).unwrap().clone();
                let args = function_call
                    .argument_ids
                    .iter()
                    .filter_map(|arg| {
                        // An argument to an `own` parameter is copied (marked in
                        // `clone_sites`), like a binding copy.
                        self.walk_entity(*arg, block)
                            .map(|node| self.maybe_clone(*arg, node))
                    })
                    .collect::<Vec<_>>();

                // `T::member()` inside a monomorphized body: dispatch directly
                // to the concrete type's member that `T` is bound to here.
                if let Some(GenericDispatch::OnConstraint(constraint_id, member_name)) = self
                    .program
                    .generic_dispatch
                    .get(&function_call.subject_id)
                    .copied()
                {
                    if let Some(&concrete_type_id) = self.current_substitution.get(&constraint_id) {
                        let own_values = self
                            .program
                            .own_generic_call_bindings
                            .get(id)
                            .cloned()
                            .unwrap_or_default();
                        // A static's trait was recorded against the ACCESSOR
                        // (the call id wasn't known at resolution).
                        let preferred = self
                            .program
                            .bound_dispatch_traits
                            .get(id)
                            .or_else(|| {
                                self.program
                                    .bound_dispatch_traits
                                    .get(&function_call.subject_id)
                            })
                            .copied();
                        if let Some(dispatch) = self.resolve_dispatch_with(
                            concrete_type_id,
                            member_name,
                            &own_values,
                            preferred,
                        ) {
                            return Some(self.emit_dispatch(dispatch, args));
                        }
                    }
                }

                // `a.member()` where `a`'s type is a trait-bounded generic `T`:
                // dispatch to the member of the concrete type `T` is bound to at this
                // monomorphization (the instance analogue of the `T::member()` path
                // above). The trait member may be abstract (bodyless), so this can't
                // fall through to a normal emit.
                if let Some(GenericDispatch::OnConstraint(constraint_id, member_name)) =
                    self.program.generic_dispatch.get(id).copied()
                {
                    if let Some(&concrete_type_id) = self.current_substitution.get(&constraint_id) {
                        let own_values = self
                            .program
                            .own_generic_call_bindings
                            .get(id)
                            .cloned()
                            .unwrap_or_default();
                        let preferred = self.program.bound_dispatch_traits.get(id).copied();
                        if let Some(dispatch) = self.resolve_dispatch_with(
                            concrete_type_id,
                            member_name,
                            &own_values,
                            preferred,
                        ) {
                            return Some(self.emit_dispatch(dispatch, args));
                        }
                    }
                }

                // A trait method re-dispatched to the receiver's concrete type: an
                // inherited default called on a concrete value (Gap E, with the
                // type recorded), or a `self`-call inside a default body (no type,
                // dispatched on the type the default is being specialized for).
                if let Some(GenericDispatch::OnType(concrete_type, member_name)) =
                    self.program.generic_dispatch.get(id).copied()
                {
                    if let Some(type_id) = concrete_type.or(self.current_self_type) {
                        if let Some(dispatch) = self.resolve_dispatch(type_id, member_name) {
                            return Some(self.emit_dispatch(dispatch, args));
                        }
                    }
                }

                let subject = self
                    .program
                    .entity_map
                    .get(&function_call.subject_id)
                    .unwrap();
                match subject {
                    Expr::Local(target_id) => {
                        let target_id = *target_id;
                        // An external std intrinsic lowers to native JS or a
                        // runtime helper.
                        if let Some(intrinsic) = self.program.intrinsics.get(&target_id).copied() {
                            return Some(self.emit_intrinsic(intrinsic, args));
                        }
                        // An `[extern]`-bound external lowers to its host (JS)
                        // import/call, method, or property access.
                        if let Some(binding) = self
                            .program
                            .external_functions
                            .get(&target_id)
                            .and_then(|external| external.extern_binding.clone())
                        {
                            let call = self.emit_extern(target_id, binding, args);
                            return Some(self.maybe_await(target_id, call));
                        }
                        // A variant constructor call builds the enum value
                        // directly: `[variant_index, ...data]` (or a native
                        // boolean for `bool`).
                        if let Some(Expr::EnumVariant(enum_id, variant_index)) =
                            self.program.entity_map.get(&target_id)
                        {
                            return Some(self.variant_value(*enum_id, *variant_index, args));
                        }
                        if target_id == self.print_fn_id {
                            return Some(js::Node::Call(
                                Box::new(js::Node::Property(
                                    Box::new(js::Node::Local("console".to_string())),
                                    "log".to_string(),
                                )),
                                args,
                            ));
                        }
                        // `List::new()` builds an empty JS array.
                        if Some(target_id) == self.list_new_fn_id {
                            return Some(js::Node::Array(Vec::new()));
                        }
                        // `list.push(x)` lowers to the native array method; the
                        // receiver is the method call's first (`self`) argument.
                        if Some(target_id) == self.list_push_fn_id {
                            let mut arguments = args.into_iter();
                            let receiver = arguments.next().unwrap_or(js::Node::Void);
                            return Some(js::Node::Call(
                                Box::new(js::Node::Property(
                                    Box::new(receiver),
                                    "push".to_string(),
                                )),
                                arguments.collect(),
                            ));
                        }
                        // `panic(msg)` lowers to a thrown error. It's wrapped in
                        // an immediately-invoked arrow so it stays valid in
                        // expression position (e.g. a match leg).
                        if Some(target_id) == self.panic_fn_id {
                            let message = args.into_iter().next().unwrap_or(js::Node::Void);
                            return Some(js::Node::Call(
                                Box::new(js::Node::Closure(js::Closure {
                                    parameters: Vec::new(),
                                    body: vec![js::Node::Throw(Box::new(message))],
                                    is_async: false,
                                })),
                                Vec::new(),
                            ));
                        }
                        // A call to a generic function/method is compiled to a
                        // specialized instance chosen by its concrete type arguments
                        // — no runtime dispatch. The binding comes from whichever
                        // channel carries it (see `call_substitution`); all feed the
                        // one `emit_instance` path. A non-generic call (no binding)
                        // is emitted as a plain function.
                        let name = match self.call_substitution(
                            *id,
                            target_id,
                            &function_call.generic_argument_ids,
                        ) {
                            Some(substitution) => self.emit_instance(target_id, &substitution),
                            None => {
                                self.ensure_function_emitted(target_id);
                                self.ng.name_for(target_id)
                            }
                        };
                        let call = js::Node::Call(Box::new(js::Node::Local(name)), args);
                        self.maybe_await(target_id, call)
                    }
                    _ => {
                        let t_subject = self.walk_entity(function_call.subject_id, block).unwrap();
                        js::Node::Call(Box::new(t_subject), args)
                    }
                }
            }
            Expr::Closure(closure_id) => {
                let closure = self.program.closures.get(closure_id).unwrap();
                let parameters = closure
                    .parameters
                    .iter()
                    .map(|parameter_id| js::Parameter {
                        name: self.ng.name_for(*parameter_id),
                    })
                    .collect::<Vec<_>>();
                let mut body = Vec::new();
                // Tuple-parameter destructures run before the body proper.
                let parameter_destructures = closure.parameter_destructures.clone();
                for destructure_id in parameter_destructures {
                    self.walk_entity(destructure_id, &mut body);
                }
                let value = self.walk_entity(closure.return_, &mut body);
                if let Some(value) = value {
                    body.push(js::Node::Return(Box::new(value)));
                }
                js::Node::Closure(js::Closure {
                    parameters,
                    body,
                    is_async: self.program.async_functions.contains(closure_id),
                })
            }
            // `async <body>` — the async-block closure invoked with no
            // arguments, yielding a promise: `(async () => { <body> })()`.
            Expr::Async(closure_id) => {
                let closure = self
                    .walk_entity(*closure_id, block)
                    .unwrap_or(js::Node::Void);
                js::Node::Call(Box::new(closure), Vec::new())
            }
            // `await <inner>`.
            Expr::Await(inner) => {
                let inner = self.walk_entity(*inner, block).unwrap_or(js::Node::Void);
                js::Node::Await(Box::new(inner))
            }
            // A bare `ret` returns void — emitted as `return;` (the emitter
            // special-cases a `Void` child).
            Expr::FunctionReturn(value) => js::Node::Return(Box::new(
                value
                    .and_then(|value| self.walk_entity(value, block))
                    .unwrap_or(js::Node::Void),
            )),
            // `a?.b.c` (proposal/try-and-lift.md §3–4): evaluate the subject
            // once; a bad tag short-circuits AS-IS; otherwise the continuation
            // runs with the binder aliased to the element, and the result is
            // wrapped back (map) or passed through (flatten).
            Expr::Lift(subject_id, binder_id, continuation_id) => {
                let subject = self
                    .walk_entity(*subject_id, block)
                    .unwrap_or(js::Node::Void);
                // A user `Lift` container: `map_instance(subject, (x) => cont)`
                // — the continuation becomes a closure whose parameter aliases
                // the binder (proposal/try-and-lift.md §4's trait path).
                if let Some(LiftDispatch::Trait {
                    member_id,
                    impl_subject,
                    subject_type_id,
                    own_generic_value,
                }) = self.program.lift_dispatch.get(&id).cloned()
                {
                    let dispatch = self.dispatch_to_member(
                        member_id,
                        impl_subject,
                        subject_type_id,
                        &[own_generic_value],
                    );
                    let Dispatch::Call(member_name, _) = dispatch else {
                        // A Lift impl's members are ordinary vilan methods.
                        return Some(js::Node::Void);
                    };
                    let parameter = self.ng.next_name();
                    self.is_bindings
                        .insert(*binder_id, js::Node::Local(parameter.clone()));
                    let mut closure_body = Vec::new();
                    let value = self
                        .walk_entity(*continuation_id, &mut closure_body)
                        .unwrap_or(js::Node::Void);
                    self.is_bindings.remove(binder_id);
                    closure_body.push(js::Node::Return(Box::new(value)));
                    return Some(js::Node::Call(
                        Box::new(js::Node::Local(member_name)),
                        vec![
                            subject,
                            js::Node::Closure(js::Closure {
                                parameters: vec![js::Parameter { name: parameter }],
                                body: closure_body,
                                is_async: false,
                            }),
                        ],
                    ));
                }
                let subject_name = self.ng.next_name();
                block.push(js::Node::ConstVariable(js::Variable {
                    name: subject_name.clone(),
                    value: Box::new(subject),
                }));
                let result_name = self.ng.next_name();
                block.push(js::Node::LetVariable(js::Variable {
                    name: result_name.clone(),
                    value: Box::new(js::Node::Null),
                }));
                let bad_body = vec![js::Node::Assignment(
                    Box::new(js::Node::Local(result_name.clone())),
                    Box::new(js::Node::Local(subject_name.clone())),
                )];
                self.is_bindings.insert(
                    *binder_id,
                    js::Node::PropertyIndex(
                        Box::new(js::Node::Local(subject_name.clone())),
                        Box::new(js::Node::Number("1".to_string(), None)),
                    ),
                );
                let mut good_body = Vec::new();
                let value = self
                    .walk_entity(*continuation_id, &mut good_body)
                    .unwrap_or(js::Node::Void);
                self.is_bindings.remove(binder_id);
                let wrapped = match self.program.lift_dispatch.get(&id) {
                    Some(LiftDispatch::Std { flatten: true, .. }) | None => value,
                    Some(LiftDispatch::Std {
                        flatten: false,
                        enum_id,
                    }) => self.variant_value(*enum_id, 0, vec![value]),
                    // Handled by the early trait-path branch above.
                    Some(LiftDispatch::Trait { .. }) => unreachable!(),
                };
                good_body.push(js::Node::Assignment(
                    Box::new(js::Node::Local(result_name.clone())),
                    Box::new(wrapped),
                ));
                block.push(js::Node::If(js::IfBranch::If(
                    Box::new(js::Node::Binary(
                        BinaryOp::Eq,
                        Box::new(js::Node::PropertyIndex(
                            Box::new(js::Node::Local(subject_name)),
                            Box::new(js::Node::Number("0".to_string(), None)),
                        )),
                        Box::new(js::Node::Number("1".to_string(), None)),
                    )),
                    bad_body,
                    Some(Box::new(js::IfBranch::Else(good_body))),
                )));
                js::Node::Local(result_name)
            }
            // Only reachable through the `Local` alias inside a continuation;
            // standalone it has no value.
            Expr::LiftBinder => js::Node::Void,
            // `expr!` (proposal/try-and-lift.md §4): evaluate the receiver once,
            // branch on the bad tag, return the bad half, yield the good half.
            Expr::TryAssert(receiver_id) => {
                let receiver = self
                    .walk_entity(*receiver_id, block)
                    .unwrap_or(js::Node::Void);
                let name = self.ng.next_name();
                block.push(js::Node::ConstVariable(js::Variable {
                    name: name.clone(),
                    value: Box::new(receiver),
                }));
                let tag_is_bad = |subject: js::Node<'src>| {
                    js::Node::Binary(
                        BinaryOp::Eq,
                        Box::new(js::Node::PropertyIndex(
                            Box::new(subject),
                            Box::new(js::Node::Number("0".to_string(), None)),
                        )),
                        Box::new(js::Node::Number("1".to_string(), None)),
                    )
                };
                match self.program.try_dispatch.get(&id).cloned() {
                    // Option/Result: the bad VALUE (`None`, `Err(e)`) is the
                    // receiver itself — return it as-is (byte-identical at any
                    // success type).
                    Some(TryDispatch::Std) | None => {
                        block.push(js::Node::If(js::IfBranch::If(
                            Box::new(tag_is_bad(js::Node::Local(name.clone()))),
                            vec![js::Node::Return(Box::new(js::Node::Local(name.clone())))],
                            None,
                        )));
                        js::Node::PropertyIndex(
                            Box::new(js::Node::Local(name)),
                            Box::new(js::Node::Number("1".to_string(), None)),
                        )
                    }
                    // A user `Try` impl: `verdict(receiver)`, branch on the
                    // Verdict tag (Good = 0, Bad = 1), return `from_bad(bad)`.
                    Some(TryDispatch::Trait {
                        verdict_id,
                        from_bad_id,
                        impl_subject,
                        receiver_type_id,
                    }) => {
                        let verdict = self.dispatch_to_member(
                            verdict_id,
                            impl_subject,
                            receiver_type_id,
                            &[],
                        );
                        let from_bad = self.dispatch_to_member(
                            from_bad_id,
                            impl_subject,
                            receiver_type_id,
                            &[],
                        );
                        let (Dispatch::Call(verdict_name, _), Dispatch::Call(from_bad_name, _)) =
                            (verdict, from_bad)
                        else {
                            // A Try impl's members are ordinary vilan methods —
                            // an intrinsic/extern here is unreachable.
                            return Some(js::Node::Void);
                        };
                        let verdict_value = self.ng.next_name();
                        block.push(js::Node::ConstVariable(js::Variable {
                            name: verdict_value.clone(),
                            value: Box::new(js::Node::Call(
                                Box::new(js::Node::Local(verdict_name)),
                                vec![js::Node::Local(name)],
                            )),
                        }));
                        block.push(js::Node::If(js::IfBranch::If(
                            Box::new(tag_is_bad(js::Node::Local(verdict_value.clone()))),
                            vec![js::Node::Return(Box::new(js::Node::Call(
                                Box::new(js::Node::Local(from_bad_name)),
                                vec![js::Node::PropertyIndex(
                                    Box::new(js::Node::Local(verdict_value.clone())),
                                    Box::new(js::Node::Number("1".to_string(), None)),
                                )],
                            )))],
                            None,
                        )));
                        js::Node::PropertyIndex(
                            Box::new(js::Node::Local(verdict_value)),
                            Box::new(js::Node::Number("1".to_string(), None)),
                        )
                    }
                }
            }
            Expr::Binary(op, lhs, rhs) => {
                let lhs = self.walk_entity(*lhs, block).unwrap_or(js::Node::Void);
                let rhs = self.walk_entity(*rhs, block).unwrap_or(js::Node::Void);
                // `x op y` where `x: T` is a trait-bounded generic: dispatch to T's
                // concrete operator impl at this monomorphization, re-resolved like
                // the instance-method generic dispatch. (`!=` negates `eq`, as below.)
                // A CONCRETE receiver whose operator method is an inherited trait
                // default (`instant < instant` over `PartialOrd`'s `lt`) records
                // `OnType` instead — same re-dispatch, the type known up front.
                if let Some(GenericDispatch::OnType(Some(receiver_type_id), member_name)) =
                    self.program.generic_dispatch.get(&id).copied()
                {
                    let concrete = self.resolve_type_id(receiver_type_id);
                    if !self.compares_natively(concrete) {
                        if let Some(dispatch) = self.resolve_dispatch(concrete, member_name) {
                            let substitution = self
                                .program
                                .method_call_substitution
                                .get(&id)
                                .cloned()
                                .unwrap_or_default();
                            let saved = self.current_substitution.clone();
                            self.current_substitution.extend(substitution);
                            let call = self.emit_dispatch(dispatch, vec![lhs.clone(), rhs.clone()]);
                            self.current_substitution = saved;
                            return Some(if matches!(*op, BinaryOp::NotEq) {
                                js::Node::Unary('!', Box::new(call))
                            } else {
                                call
                            });
                        }
                    }
                }
                if let Some(GenericDispatch::OnConstraint(constraint_id, member_name)) =
                    self.program.generic_dispatch.get(&id).copied()
                {
                    let concrete = self
                        .current_substitution
                        .get(&constraint_id)
                        .map(|type_id| self.resolve_type_id(*type_id));
                    // A native-equality concrete type (`Option<i32>`'s element) keeps
                    // native `===`/`!==`; only an aggregate (`Option<Point>`)
                    // dispatches to its `eq` impl.
                    if let Some(concrete_type_id) = concrete.filter(|t| !self.compares_natively(*t))
                    {
                        if let Some(dispatch) = self.resolve_dispatch(concrete_type_id, member_name)
                        {
                            let call = self.emit_dispatch(dispatch, vec![lhs, rhs]);
                            return Some(if matches!(*op, BinaryOp::NotEq) {
                                js::Node::Unary('!', Box::new(call))
                            } else {
                                call
                            });
                        }
                    }
                }
                // An overloaded operator (`a + b` where `a`'s type implements
                // `Add`) compiles to the trait method call `add(a, b)`. On a generic
                // receiver (`Option<Point> ==`) the method is monomorphized against
                // the recorded type-arg substitution so its body specializes.
                if let Some(&method_id) = self.program.binary_op_dispatch.get(&id) {
                    let name = if let Some(substitution) =
                        self.program.method_call_substitution.get(&id)
                    {
                        let substitution = substitution.clone();
                        self.emit_instance(method_id, &substitution)
                    } else {
                        self.ensure_function_emitted(method_id);
                        self.ng.name_for(method_id)
                    };
                    let call = js::Node::Call(Box::new(js::Node::Local(name)), vec![lhs, rhs]);
                    // `a != b` dispatches to `eq` and negates — the impl provides
                    // `eq`, and `ne` is just its `!eq` default.
                    return Some(if matches!(*op, BinaryOp::NotEq) {
                        js::Node::Unary('!', Box::new(call))
                    } else {
                        call
                    });
                }
                // JS bitwise is signed: on `u32` operands `>>` must be the
                // logical `>>>`, and the value-producing ops re-wrap with
                // `>>> 0` so a set high bit stays a large unsigned value
                // instead of going negative. `i32` keeps the native ops (JS
                // ToInt32 IS i32 semantics), and `BigInt` must NOT wrap
                // (arbitrary precision). Proposal/bits-and-bytes.md §2.
                if matches!(
                    op,
                    BinaryOp::Shl
                        | BinaryOp::Shr
                        | BinaryOp::BitAnd
                        | BinaryOp::BitXor
                        | BinaryOp::BitOr
                ) && self.binary_operands_are_u32(id)
                {
                    if matches!(op, BinaryOp::Shr) {
                        return Some(binary(BinaryOp::UShr, lhs, rhs));
                    }
                    return Some(binary(
                        BinaryOp::UShr,
                        binary(*op, lhs, rhs),
                        js::Node::Number("0".to_string(), None),
                    ));
                }
                // Integer division truncates toward zero
                // (proposal/numeric-types.md §2): `Math.trunc(a / b)`.
                // Float and BigInt division stay native.
                if matches!(op, BinaryOp::Div) && self.binary_operands_are_integer(id) {
                    return Some(js::Node::Call(
                        Box::new(js::Node::Local("Math.trunc".to_string())),
                        vec![binary(*op, lhs, rhs)],
                    ));
                }
                binary(*op, lhs, rhs)
            }
            Expr::Unary(operator, operand) => {
                let operand = self.walk_entity(*operand, block).unwrap_or(js::Node::Void);
                js::Node::Unary(*operator, Box::new(operand))
            }
            // A view of a scalar place lowers to a `[base, key]` pair — a boxed
            // local's cell at slot 0, or a struct's field slot. A view of an
            // aggregate is the value's own JS reference (an aggregate is its own
            // view), so it passes through unchanged.
            Expr::Reference(operand, _) => {
                if self.emits_scalar_view_ref(id, *operand) {
                    let (base, key) = match self.program.entity_map.get(operand) {
                        Some(Expr::Field(subject, _, field_index)) => (
                            self.walk_entity(*subject, block).unwrap_or(js::Node::Void),
                            js::Node::Number(field_index.to_string(), None),
                        ),
                        // `&mut list[i]` — the checked mint (`__at_view`): the
                        // scalar `(base, key)` pair exists only for an in-bounds
                        // element, so a view of an absent element panics at the
                        // `&mut`, not at first use through it.
                        Some(Expr::Index(subject, index)) => {
                            let base = self.walk_entity(*subject, block).unwrap_or(js::Node::Void);
                            let key = self.walk_entity(*index, block).unwrap_or(js::Node::Void);
                            self.used_helpers.insert("__at_view");
                            return Some(js::Node::Call(
                                Box::new(js::Node::Local("__at_view".to_string())),
                                vec![base, key],
                            ));
                        }
                        // A boxed scalar local: the cell itself (slot 0 holds the
                        // value), not the `[0]` read `walk_entity` would produce.
                        Some(Expr::Local(root)) => (
                            js::Node::Local(self.ng.name_for(*root)),
                            js::Node::Number("0".to_string(), None),
                        ),
                        _ => (
                            self.walk_entity(*operand, block).unwrap_or(js::Node::Void),
                            js::Node::Number("0".to_string(), None),
                        ),
                    };
                    return Some(js::Node::Array(vec![base, key]));
                }
                return self.walk_entity(*operand, block);
            }
            // Deref of an aggregate view is the operand itself; deref of a scalar
            // `(base, key)` view reads/writes through `operand[0][operand[1]]`.
            Expr::Dereference(operand) => {
                let value = self.walk_entity(*operand, block);
                if self.derefs_scalar_view(*operand) {
                    let mut view = value.unwrap_or(js::Node::Void);
                    // A view produced by a call (`*obj.slot()`) is bound to a temp
                    // first, so the `[0]` and `[1]` reads don't evaluate the call
                    // twice; a plain binding / reference is cheap to repeat.
                    if matches!(self.program.entity_map.get(operand), Some(Expr::Call(..))) {
                        let name = self.ng.next_name();
                        block.push(js::Node::ConstVariable(js::Variable {
                            name: name.clone(),
                            value: Box::new(view),
                        }));
                        view = js::Node::Local(name);
                    }
                    let base = js::Node::PropertyIndex(
                        Box::new(view.clone()),
                        Box::new(js::Node::Number("0".to_string(), None)),
                    );
                    let key = js::Node::PropertyIndex(
                        Box::new(view),
                        Box::new(js::Node::Number("1".to_string(), None)),
                    );
                    return Some(js::Node::PropertyIndex(Box::new(base), Box::new(key)));
                }
                return value;
            }
            Expr::Variable(id) => {
                if self
                    .program
                    .reference_count
                    .get(id)
                    .map(|x| *x < 1)
                    .unwrap_or(true)
                {
                    // An unused binding is dropped — but a side-effecting
                    // initializer (a call mutating through `&mut`, say) must still
                    // run; emit it as a bare statement, discarding the value.
                    let initial = self.program.variables.get(id).and_then(|v| v.initial);
                    if let Some(value_id) = initial
                        && self.expr_has_side_effects(value_id)
                    {
                        return self.walk_entity(value_id, block);
                    }
                    return None;
                }
                let name = self.ng.name_for(*id);
                let variable = self.program.variables.get(id).unwrap();
                let value = variable
                    .initial
                    .and_then(|value_id| {
                        self.walk_entity(value_id, block)
                            .map(|node| self.maybe_clone(value_id, node))
                    })
                    .unwrap_or(js::Node::Void);
                // A boxed scalar local is declared as a one-slot cell.
                let value = if self.local_is_boxed(*id) {
                    js::Node::Array(vec![value])
                } else {
                    value
                };
                let js_variable = js::Variable {
                    name,
                    value: Box::new(value),
                };
                if variable.mutable {
                    js::Node::LetVariable(js_variable)
                } else {
                    js::Node::ConstVariable(js_variable)
                }
            }
            Expr::Assignment(target_id, value_id) => {
                let value = self.walk_entity(*value_id, block).unwrap_or(js::Node::Void);
                let value = self.maybe_clone(*value_id, value);
                // Writing a *whole value* through a view. A `Shared` write is a
                // single-slot view (`cell.v`): rebind the slot, so every handle to
                // the cell sees the new value (`cell.v = value`). An ordinary
                // aggregate view copies the fields in place (`Object.assign`), so
                // the target and its aliases update rather than rebinding a local.
                // A primitive view's `*c` is a `[0]` slot write — the normal path.
                if let Some(Expr::Dereference(operand)) = self.program.entity_map.get(target_id) {
                    if self.is_shared_write(*operand) {
                        let slot = self.walk_entity(*operand, block).unwrap_or(js::Node::Void);
                        return Some(js::Node::Assignment(Box::new(slot), Box::new(value)));
                    }
                    if !self.derefs_scalar_view(*operand) {
                        let base = self.walk_entity(*operand, block).unwrap_or(js::Node::Void);
                        return Some(js::Node::Call(
                            Box::new(js::Node::Local("Object.assign".to_string())),
                            vec![base, value],
                        ));
                    }
                }
                // `pair.0 = v` on a multi-slot (tuple-typed) element: write
                // each slot of the region from the value (evaluated once).
                // Statically-known width keeps this plain slot assignments —
                // the const-eval interpreter runs them like any other write.
                if let Some(&Expr::TupleIndex(subject_id, offset, width)) =
                    self.program.entity_map.get(target_id)
                {
                    if width > 1 {
                        let subject = self
                            .walk_entity(subject_id, block)
                            .unwrap_or(js::Node::Void);
                        let value_name = self.ng.next_name();
                        block.push(js::Node::ConstVariable(js::Variable {
                            name: value_name.clone(),
                            value: Box::new(value),
                        }));
                        for slot in 0..width {
                            block.push(js::Node::Assignment(
                                Box::new(js::Node::PropertyIndex(
                                    Box::new(subject.clone()),
                                    Box::new(js::Node::Number((offset + slot).to_string(), None)),
                                )),
                                Box::new(js::Node::PropertyIndex(
                                    Box::new(js::Node::Local(value_name.clone())),
                                    Box::new(js::Node::Number(slot.to_string(), None)),
                                )),
                            ));
                        }
                        return None;
                    }
                }
                // `list[i] = v` — the checked write (`__at_put`): writing never
                // creates a slot (growth is `push`), so an out-of-bounds write
                // panics. The read side is `__at`; an assignment target can't
                // be a call, so the write gets its own helper.
                if let Some(&Expr::Index(subject_id, index_id)) =
                    self.program.entity_map.get(target_id)
                {
                    let subject = self
                        .walk_entity(subject_id, block)
                        .unwrap_or(js::Node::Void);
                    let index = self.walk_entity(index_id, block).unwrap_or(js::Node::Void);
                    self.used_helpers.insert("__at_put");
                    return Some(js::Node::Call(
                        Box::new(js::Node::Local("__at_put".to_string())),
                        vec![subject, index, value],
                    ));
                }
                let target = self
                    .walk_entity(*target_id, block)
                    .unwrap_or(js::Node::Void);
                js::Node::Assignment(Box::new(target), Box::new(value))
            }
            Expr::Parameter(_) => {
                return None;
            }
            Expr::Block(body) => {
                for statement in &body.0 {
                    if let Some(node) = self.walk_entity(*statement, block) {
                        // A statement that lowered to nothing (a void tail, a
                        // self-emitting loop/`if`) leaves no stray `undefined`.
                        if !matches!(node, js::Node::Void) {
                            block.push(node);
                        }
                    }
                }
                return self.walk_entity(body.1, block);
            }
            Expr::For(condition, body) => {
                // Every loop compiles to a `while`; an absent condition is an
                // infinite loop, i.e. `while (true)`.
                let t_condition = condition
                    .and_then(|condition| self.walk_entity(condition, block))
                    .unwrap_or(js::Node::Bool(true));
                let mut t_body = self.walk_list(&body.0);
                match self.program.entity_map.get(&body.1) {
                    Some(Expr::Void) | None => {}
                    Some(_) => {
                        if let Some(node) = self.walk_entity(body.1, &mut t_body) {
                            if !matches!(node, js::Node::Void) {
                                t_body.push(node);
                            }
                        }
                    }
                }
                // A loop is a statement with no value: emit it into the block
                // and yield void, so a loop as a block's tail isn't treated as
                // the block's result.
                block.push(js::Node::While(Box::new(t_condition), t_body));
                js::Node::Void
            }
            Expr::ForEach(iterable_id, item_id, body) => {
                let t_iterable = self
                    .walk_entity(*iterable_id, block)
                    .unwrap_or(js::Node::Void);
                // `Set` is a vilan struct over a `NativeMap`; iterate the backing
                // map's stored originals (`set[0].values()`), in insertion order.
                let t_iterable = if self.is_set_typed(*iterable_id) {
                    self.used_helpers.insert("__set_iter");
                    js::Node::Call(
                        Box::new(js::Node::Local("__set_iter".to_string())),
                        vec![t_iterable],
                    )
                } else {
                    t_iterable
                };

                if let Some(&next_id) = self.program.for_each_next.get(&id) {
                    // Iterator protocol: evaluate the iterator once, then loop
                    // calling `next()` until it yields `None` (variant 1); the
                    // `Some` payload (slot 1) is the element.
                    self.ensure_function_emitted(next_id);
                    let next_name = self.ng.name_for(next_id);
                    let iterator_name = self.ng.next_name();
                    let next_value_name = self.ng.next_name();
                    block.push(js::Node::ConstVariable(js::Variable {
                        name: iterator_name.clone(),
                        value: Box::new(t_iterable),
                    }));
                    let mut loop_body = vec![
                        js::Node::ConstVariable(js::Variable {
                            name: next_value_name.clone(),
                            value: Box::new(js::Node::Call(
                                Box::new(js::Node::Local(next_name)),
                                vec![js::Node::Local(iterator_name.clone())],
                            )),
                        }),
                        js::Node::If(js::IfBranch::If(
                            Box::new(js::Node::Binary(
                                BinaryOp::NotEq,
                                Box::new(js::Node::PropertyIndex(
                                    Box::new(js::Node::Local(next_value_name.clone())),
                                    Box::new(js::Node::Number("0".to_string(), None)),
                                )),
                                Box::new(js::Node::Number("0".to_string(), None)),
                            )),
                            vec![js::Node::Break],
                            None,
                        )),
                    ];
                    if let Some(item_id) = item_id {
                        loop_body.push(js::Node::ConstVariable(js::Variable {
                            name: self.ng.name_for(*item_id),
                            value: Box::new(js::Node::PropertyIndex(
                                Box::new(js::Node::Local(next_value_name)),
                                Box::new(js::Node::Number("1".to_string(), None)),
                            )),
                        }));
                    }
                    loop_body.extend(self.walk_list(&body.0));
                    if let Some(Expr::Void) | None = self.program.entity_map.get(&body.1) {
                    } else if let Some(node) = self.walk_entity(body.1, &mut loop_body) {
                        if !matches!(node, js::Node::Void) {
                            loop_body.push(node);
                        }
                    }
                    block.push(js::Node::While(Box::new(js::Node::Bool(true)), loop_body));
                    return Some(js::Node::Void);
                }

                // `for e in &mut list` / `&list` — an indexed loop binding each
                // element as a view: a scalar element pairs to `[list, i]`, an
                // aggregate is `list[i]` (its own reference). `list.keys()` yields
                // the indices. The list is bound to a temp so it's evaluated once.
                if let Some(item_id) = *item_id
                    && self.program.for_each_views.contains_key(&item_id)
                {
                    let list_name = self.ng.next_name();
                    block.push(js::Node::ConstVariable(js::Variable {
                        name: list_name.clone(),
                        value: Box::new(t_iterable),
                    }));
                    let index_name = self.ng.next_name();
                    let element = if self.program.primitive_views.contains(&item_id) {
                        js::Node::Array(vec![
                            js::Node::Local(list_name.clone()),
                            js::Node::Local(index_name.clone()),
                        ])
                    } else {
                        js::Node::PropertyIndex(
                            Box::new(js::Node::Local(list_name.clone())),
                            Box::new(js::Node::Local(index_name.clone())),
                        )
                    };
                    let mut loop_body = vec![js::Node::ConstVariable(js::Variable {
                        name: self.ng.name_for(item_id),
                        value: Box::new(element),
                    })];
                    loop_body.extend(self.walk_list(&body.0));
                    if let Some(Expr::Void) | None = self.program.entity_map.get(&body.1) {
                    } else if let Some(node) = self.walk_entity(body.1, &mut loop_body)
                        && !matches!(node, js::Node::Void)
                    {
                        loop_body.push(node);
                    }
                    let keys = js::Node::Call(
                        Box::new(js::Node::Property(
                            Box::new(js::Node::Local(list_name)),
                            "keys".to_string(),
                        )),
                        Vec::new(),
                    );
                    block.push(js::Node::ForOf(index_name, Box::new(keys), loop_body));
                    return Some(js::Node::Void);
                }

                // Otherwise a native `for...of` (a `List` is a JS array).
                let binding = item_id
                    .map(|item_id| self.ng.name_for(item_id))
                    .unwrap_or_else(|| "_".to_string());
                let mut t_body = self.walk_list(&body.0);
                if let Some(Expr::Void) | None = self.program.entity_map.get(&body.1) {
                } else if let Some(node) = self.walk_entity(body.1, &mut t_body) {
                    if !matches!(node, js::Node::Void) {
                        t_body.push(node);
                    }
                }
                block.push(js::Node::ForOf(binding, Box::new(t_iterable), t_body));
                js::Node::Void
            }
            Expr::Jump(target) => match *target {
                "break" => js::Node::Break,
                "continue" => js::Node::Continue,
                _ => js::Node::Void,
            },
            Expr::If(branch) => {
                fn walk_branch<'src>(
                    t: &mut Transformer<'src>,
                    branch: &ExprIfBranch,
                    block: &mut Vec<js::Node<'src>>,
                    expr_variable_name: &mut Option<String>,
                ) -> js::IfBranch<'src> {
                    match branch {
                        ExprIfBranch::If(condition, body, else_) => {
                            let t_condition = t
                                .walk_entity(*condition, block)
                                .unwrap_or(js::Node::Bool(false));
                            let mut t_body = t.walk_list(&body.0);
                            let body_expr = t.program.entity_map.get(&body.1);
                            match body_expr {
                                None => {}
                                Some(Expr::Void) => {}
                                Some(_) => {
                                    let t_block_expr = t.walk_entity(body.1, &mut t_body);
                                    let variable_name =
                                        expr_variable_name.get_or_insert_with(|| t.ng.next_name());
                                    let value = t_block_expr.unwrap_or(js::Node::Null);
                                    let variable_name = variable_name.clone();
                                    t.push_result_or_divergence(&variable_name, value, &mut t_body);
                                }
                            }
                            js::IfBranch::If(
                                Box::new(t_condition),
                                t_body,
                                else_.as_ref().map(|x| {
                                    Box::new(walk_branch(t, x, block, expr_variable_name))
                                }),
                            )
                        }
                        ExprIfBranch::Else(body) => {
                            let mut t_body = t.walk_list(&body.0);
                            let body_expr = t.program.entity_map.get(&body.1);
                            match body_expr {
                                None => {}
                                Some(Expr::Void) => {}
                                Some(_) => {
                                    let t_block_expr = t.walk_entity(body.1, &mut t_body);
                                    let variable_name =
                                        expr_variable_name.get_or_insert_with(|| t.ng.next_name());
                                    let value = t_block_expr.unwrap_or(js::Node::Null);
                                    let variable_name = variable_name.clone();
                                    t.push_result_or_divergence(&variable_name, value, &mut t_body);
                                }
                            }
                            js::IfBranch::Else(t_body)
                        }
                    }
                }
                let mut expr_variable_name = None;
                let branch = walk_branch(self, branch, block, &mut expr_variable_name);
                match expr_variable_name {
                    Some(variable_name) => {
                        let expr_variable = js::Node::LetVariable(js::Variable {
                            name: variable_name.clone(),
                            value: Box::new(js::Node::Null),
                        });
                        block.push(expr_variable);
                        block.push(js::Node::If(branch));
                        js::Node::Local(variable_name)
                    }
                    // A value-less `if` (no branch produces a value) is a
                    // statement: emit it into the block and yield void, so a
                    // trailing `if` isn't mistaken for the block's/function's
                    // result (and wrapped in `return`/`process.exit`).
                    None => {
                        block.push(js::Node::If(branch));
                        js::Node::Void
                    }
                }
            }
            Expr::Is(subject_id, pattern) => {
                // Evaluate the subject once into a temp; the test reads from it,
                // and any captures alias its payload slots.
                let t_subject = self
                    .walk_entity(*subject_id, block)
                    .unwrap_or(js::Node::Void);
                let subject_name = self.ng.next_name();
                block.push(js::Node::ConstVariable(js::Variable {
                    name: subject_name.clone(),
                    value: Box::new(t_subject),
                }));
                let mut conditions = Vec::new();
                self.compile_is_pattern(pattern, js::Node::Local(subject_name), &mut conditions);
                // An irrefutable pattern (binding/wildcard/tuple) is always true.
                conditions
                    .into_iter()
                    .reduce(|a, b| js::Node::Binary(BinaryOp::And, Box::new(a), Box::new(b)))
                    .unwrap_or(js::Node::Bool(true))
            }
            Expr::Destructure(value_id, pattern) => {
                // `let (a, b) = value` -> bind the value to a temp (evaluated
                // once), then declare each binding from a positional slot:
                // `const __d = value; const a = __d[0]; const b = __d[1];`. An
                // irrefutable binder produces no conditions.
                let value = self.walk_entity(*value_id, block).unwrap_or(js::Node::Void);
                let temp_name = self.ng.next_name();
                block.push(js::Node::ConstVariable(js::Variable {
                    name: temp_name.clone(),
                    value: Box::new(value),
                }));
                let mut conditions = Vec::new();
                let mut bindings = Vec::new();
                self.compile_pattern(
                    pattern,
                    js::Node::Local(temp_name),
                    &mut conditions,
                    &mut bindings,
                );
                for binding in bindings {
                    block.push(binding);
                }
                return None;
            }
            Expr::Match(subject_id, legs) => {
                let t_subject = self
                    .walk_entity(*subject_id, block)
                    .unwrap_or(js::Node::Void);
                // Evaluate the subject once into a temporary; every variant
                // test and capture reads from it.
                let subject_name = self.ng.next_name();
                block.push(js::Node::ConstVariable(js::Variable {
                    name: subject_name.clone(),
                    value: Box::new(t_subject),
                }));
                let result_name = self.ng.next_name();
                block.push(js::Node::LetVariable(js::Variable {
                    name: result_name.clone(),
                    value: Box::new(js::Node::Null),
                }));
                // Each leg becomes an optional variant test plus a body that
                // declares its captures and assigns the leg's value.
                let mut compiled_legs: Vec<(Option<js::Node<'src>>, Vec<js::Node<'src>>)> =
                    Vec::new();
                for leg in legs {
                    let mut leg_body = Vec::new();
                    let subject = js::Node::Local(subject_name.clone());
                    let condition = if leg.guard.is_none() {
                        // No guard: captures are declared as `const`s in the body.
                        let mut conditions = Vec::new();
                        self.compile_pattern(&leg.pattern, subject, &mut conditions, &mut leg_body);
                        conditions.into_iter().reduce(|a, b| {
                            js::Node::Binary(BinaryOp::And, Box::new(a), Box::new(b))
                        })
                    } else {
                        // Guarded: the guard reads the pattern's captures, so they
                        // can't be `const`s declared inside the matched body — alias
                        // them to the subject's slots (like an `is` test) for the
                        // guard and body, then clear the aliases after this leg.
                        let captures = Self::pattern_capture_ids(&leg.pattern);
                        let mut conditions = Vec::new();
                        self.compile_is_pattern(&leg.pattern, subject, &mut conditions);
                        let mut guard_block = Vec::new();
                        if let Some(guard) = self.walk_entity(leg.guard.unwrap(), &mut guard_block)
                        {
                            conditions.push(guard);
                        }
                        let condition = conditions.into_iter().reduce(|a, b| {
                            js::Node::Binary(BinaryOp::And, Box::new(a), Box::new(b))
                        });
                        let value = self.walk_entity(leg.body, &mut leg_body);
                        let value = value.unwrap_or(js::Node::Null);
                        self.push_result_or_divergence(&result_name, value, &mut leg_body);
                        for capture in captures {
                            self.is_bindings.remove(&capture);
                        }
                        let is_catch_all = condition.is_none();
                        compiled_legs.push((condition, leg_body));
                        if is_catch_all {
                            break;
                        }
                        continue;
                    };
                    let value = self.walk_entity(leg.body, &mut leg_body);
                    let value = value.unwrap_or(js::Node::Null);
                    self.push_result_or_divergence(&result_name, value, &mut leg_body);
                    let is_catch_all = condition.is_none();
                    compiled_legs.push((condition, leg_body));
                    if is_catch_all {
                        // Later legs are unreachable.
                        break;
                    }
                }
                // The analyzer verified exhaustiveness, so the final leg can
                // always be the `else` branch.
                if let Some(last_leg) = compiled_legs.last_mut() {
                    last_leg.0 = None;
                }
                let mut chain: Option<js::IfBranch<'src>> = None;
                for (condition, leg_body) in compiled_legs.into_iter().rev() {
                    chain = Some(match condition {
                        None => js::IfBranch::Else(leg_body),
                        Some(condition) => {
                            js::IfBranch::If(Box::new(condition), leg_body, chain.map(Box::new))
                        }
                    });
                }
                match chain {
                    // A lone catch-all needs no branching at all.
                    Some(js::IfBranch::Else(leg_body)) => block.extend(leg_body),
                    Some(chain) => block.push(js::Node::If(chain)),
                    None => {}
                }
                js::Node::Local(result_name)
            }
            Expr::List(ids) => {
                let items = ids
                    .iter()
                    .filter_map(|id| self.walk_entity(*id, block))
                    .collect();
                js::Node::Array(items)
            }
            Expr::Tuple(ids) => {
                // Tuples store flat: a tuple-typed element's value is itself a flat
                // array, so splice its slots in (`...elem`) rather than nesting it.
                let items = ids
                    .iter()
                    .filter_map(|id| {
                        let value = self.walk_entity(*id, block)?;
                        Some(if self.is_tuple_typed(*id) {
                            js::Node::Spread(Box::new(value))
                        } else {
                            value
                        })
                    })
                    .collect();
                js::Node::Array(items)
            }
            Expr::StructInitializer(_struct_id, assignments) => {
                // let struct_ = self.program.structs.get(struct_id).unwrap();
                // let mut properties_ng = NameGenerator::simple(debug_names);
                let mut properties = assignments
                    .iter()
                    .filter_map(|(i, id)| {
                        // let field = struct_.fields.get(*i).unwrap();
                        let value = self.walk_entity(*id, block);
                        value.map(|x| (i, x))
                    })
                    .collect::<Vec<_>>();
                properties.sort_by(|a, b| a.0.cmp(b.0));
                let items = properties.into_iter().map(|x| x.1).collect::<Vec<_>>();
                js::Node::Array(items)
            }
            Expr::Module(_module_id) => {
                // println!("SEEN MODULE");
                // let module = self.program.modules.get(module_id).expect("failed to find module by id");
                // self.walk_entities(&module.body.0, block);
                return None;
            }
        })
    }

    /// The JS value for an enum variant. `bool` lowers to a native boolean
    /// (`false`/`true`), a numeric (C-like) enum to its integer discriminant, and
    /// every other enum to an array `[index, ...data]`.
    fn variant_value(
        &self,
        enum_id: Id,
        variant_index: usize,
        data: Vec<js::Node<'src>>,
    ) -> js::Node<'src> {
        if Some(enum_id) == self.program.bool_enum_id {
            return js::Node::Bool(variant_index == 1);
        }
        if let Some(discriminant) = self.numeric_enum_discriminant(enum_id, variant_index) {
            return js::Node::Number(discriminant.to_string(), None);
        }
        let mut items = vec![js::Node::Number(variant_index.to_string(), None)];
        items.extend(data);
        js::Node::Array(items)
    }

    /// The integer discriminant of a variant if `enum_id` is a numeric (C-like)
    /// enum, else `None` (it uses the array representation).
    fn numeric_enum_discriminant(&self, enum_id: Id, variant_index: usize) -> Option<i64> {
        let enum_ = self.program.enums.get(&enum_id)?;
        if !enum_.is_numeric {
            return None;
        }
        enum_
            .variants
            .get(variant_index)
            .map(|variant| variant.discriminant)
    }

    /// For a variant of an enum that lowers to a native scalar — `bool`
    /// (`subject === true`) or a numeric enum (`subject === discriminant`) — the
    /// equality test. `None` for array-form enums, which test the `[0]` slot.
    fn scalar_variant_test(
        &self,
        enum_id: Id,
        variant_index: usize,
        subject: &js::Node<'src>,
    ) -> Option<js::Node<'src>> {
        let value = if Some(enum_id) == self.program.bool_enum_id {
            js::Node::Bool(variant_index == 1)
        } else {
            js::Node::Number(
                self.numeric_enum_discriminant(enum_id, variant_index)?
                    .to_string(),
                None,
            )
        };
        Some(js::Node::Binary(
            BinaryOp::Eq,
            Box::new(subject.clone()),
            Box::new(value),
        ))
    }

    // Compiles a match pattern against the JS expression holding the value it
    // matches: variant tests are appended to `conditions` and capture
    // declarations to `bindings`.
    /// Compiles a pattern for an `is` test: collects the boolean test conditions
    /// and records each capture as an alias to the subject's payload slot (so
    /// references compile to `t[i]` rather than a binding statement).
    fn compile_is_pattern(
        &mut self,
        pattern: &ExprPattern,
        subject: js::Node<'src>,
        conditions: &mut Vec<js::Node<'src>>,
    ) {
        match pattern {
            ExprPattern::Wildcard => {}
            ExprPattern::Binding(capture_id) => {
                self.is_bindings.insert(*capture_id, subject);
            }
            ExprPattern::Variant(enum_id, variant_index, payload) => {
                // `bool` and numeric enums lower to native values (see
                // `compile_pattern`), so they test by value, not array slot.
                if let Some(test) = self.scalar_variant_test(*enum_id, *variant_index, &subject) {
                    conditions.push(test);
                    return;
                }
                conditions.push(js::Node::Binary(
                    BinaryOp::Eq,
                    Box::new(js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number("0".to_string(), None)),
                    )),
                    Box::new(js::Node::Number(variant_index.to_string(), None)),
                ));
                for (data_index, sub_pattern) in payload.iter().enumerate() {
                    let element = js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number((data_index + 1).to_string(), None)),
                    );
                    self.compile_is_pattern(sub_pattern, element, conditions);
                }
            }
            ExprPattern::Tuple(elements) => {
                let mut leaves = Vec::new();
                Self::flatten_tuple_pattern(elements, &subject, 0, &mut leaves);
                for (sub_pattern, element) in leaves {
                    self.compile_is_pattern(sub_pattern, element, conditions);
                }
            }
            ExprPattern::Literal(literal_id) => {
                conditions.push(self.literal_equality(*literal_id, subject));
            }
        }
    }

    /// Flattens a tuple pattern's elements to `(sub-pattern, subject-slot)` leaves
    /// for flat storage: a nested tuple pattern recurses (accumulating the flat
    /// offset), a width-1 element reads `subject[offset]`, and a multi-slot capture
    /// (a binding/wildcard of tuple type) reslices `subject.slice(offset, end)`.
    fn flatten_tuple_pattern<'a>(
        elements: &'a [(ExprPattern, usize)],
        subject: &js::Node<'src>,
        base: usize,
        out: &mut Vec<(&'a ExprPattern, js::Node<'src>)>,
    ) {
        let mut offset = base;
        for (sub_pattern, width) in elements {
            match sub_pattern {
                ExprPattern::Tuple(inner) => {
                    Self::flatten_tuple_pattern(inner, subject, offset, out);
                }
                _ if *width == 1 => out.push((
                    sub_pattern,
                    js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number(offset.to_string(), None)),
                    ),
                )),
                _ => out.push((
                    sub_pattern,
                    js::Node::Call(
                        Box::new(js::Node::Property(
                            Box::new(subject.clone()),
                            "slice".to_string(),
                        )),
                        vec![
                            js::Node::Number(offset.to_string(), None),
                            js::Node::Number((offset + width).to_string(), None),
                        ],
                    ),
                )),
            }
            offset += width;
        }
    }

    /// Lowers an `[extern]`-bound call to its host (JS) form. The first argument
    /// is the receiver for method/property bindings; a `Function` binding with a
    /// module records the import to emit.
    fn emit_extern(
        &mut self,
        target_id: Id,
        binding: ExternBinding<'src>,
        args: Vec<js::Node<'src>>,
    ) -> js::Node<'src> {
        match binding {
            ExternBinding::Function { module, symbol } => {
                if let Some(module) = module {
                    self.used_imports
                        .entry(module.to_string())
                        .or_default()
                        .insert(symbol.to_string());
                }
                // A `__`-named free extern is a runtime helper whose source
                // lives in the helper table (glue for shapes the extern binding
                // forms can't express — option-object arguments, `??`
                // flattening, global property reads).
                if let Some(helper) = extern_helper(symbol) {
                    self.used_helpers.insert(helper);
                }
                js::Node::Call(Box::new(js::Node::Local(symbol.to_string())), args)
            }
            ExternBinding::Method { symbol } => {
                // The JS method name defaults to the external's source name.
                let method = symbol
                    .or_else(|| {
                        self.program
                            .external_functions
                            .get(&target_id)
                            .map(|e| e.name)
                    })
                    .unwrap_or("")
                    .to_string();
                let mut args = args.into_iter();
                let receiver = args.next().unwrap_or(js::Node::Void);
                js::Node::Call(
                    Box::new(js::Node::Property(Box::new(receiver), method)),
                    args.collect(),
                )
            }
            ExternBinding::Get { symbol } => {
                let receiver = args.into_iter().next().unwrap_or(js::Node::Void);
                js::Node::Property(Box::new(receiver), symbol.to_string())
            }
            ExternBinding::Set { symbol } => {
                let mut args = args.into_iter();
                let receiver = args.next().unwrap_or(js::Node::Void);
                let value = args.next().unwrap_or(js::Node::Void);
                js::Node::Assignment(
                    Box::new(js::Node::Property(Box::new(receiver), symbol.to_string())),
                    Box::new(value),
                )
            }
            // `new Symbol(args)` — the callee renders verbatim, and an extern is
            // only ever emitted as a direct call, so the textual form is exact.
            // A module-qualified class imports first (the `Function` rule).
            ExternBinding::New { module, symbol } => {
                if let Some(module) = module {
                    self.used_imports
                        .entry(module.to_string())
                        .or_default()
                        .insert(symbol.to_string());
                }
                js::Node::Call(Box::new(js::Node::Local(format!("new {symbol}"))), args)
            }
        }
    }

    /// Lowers an `external` std intrinsic call. Method intrinsics take the
    /// receiver as the first argument; helper-backed ones record the helper so
    /// it's emitted in the prelude.
    fn emit_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        args: Vec<js::Node<'src>>,
    ) -> js::Node<'src> {
        // A method that maps directly onto a native JS method (`str`, `Set`, `Map`):
        // the receiver is `self` (the first argument), the rest pass through as args.
        fn native_method<'a, I: Iterator<Item = js::Node<'a>>>(
            args: &mut I,
            native: &str,
        ) -> js::Node<'a> {
            let receiver = args.next().unwrap_or(js::Node::Void);
            js::Node::Call(
                Box::new(js::Node::Property(Box::new(receiver), native.to_string())),
                args.collect(),
            )
        }
        let mut args = args.into_iter();
        match intrinsic {
            Intrinsic::Scan => {
                self.used_helpers.insert("__scan");
                js::Node::Call(Box::new(js::Node::Local("__scan".to_string())), Vec::new())
            }
            Intrinsic::StrTrim => native_method(&mut args, "trim"),
            Intrinsic::StrToLowercaseAscii => native_method(&mut args, "toLowerCase"),
            Intrinsic::StrToUppercase => native_method(&mut args, "toUpperCase"),
            Intrinsic::StrContains => native_method(&mut args, "includes"),
            Intrinsic::StrStartsWith => native_method(&mut args, "startsWith"),
            Intrinsic::StrEndsWith => native_method(&mut args, "endsWith"),
            Intrinsic::StrReplace => native_method(&mut args, "replaceAll"),
            Intrinsic::StrRepeat => native_method(&mut args, "repeat"),
            Intrinsic::StrSplit => native_method(&mut args, "split"),
            Intrinsic::StrSubstring => native_method(&mut args, "substring"),
            Intrinsic::StrLen | Intrinsic::ListLen => js::Node::Property(
                Box::new(args.next().unwrap_or(js::Node::Void)),
                "length".to_string(),
            ),
            Intrinsic::ListGet => {
                self.used_helpers.insert("__list_get");
                self.used_helpers.insert("__clone");
                js::Node::Call(
                    Box::new(js::Node::Local("__list_get".to_string())),
                    args.collect(),
                )
            }
            Intrinsic::ListPop => {
                self.used_helpers.insert("__list_pop");
                js::Node::Call(
                    Box::new(js::Node::Local("__list_pop".to_string())),
                    args.collect(),
                )
            }
            Intrinsic::ParseI32 => {
                self.used_helpers.insert("__parse_i32");
                js::Node::Call(
                    Box::new(js::Node::Local("__parse_i32".to_string())),
                    vec![args.next().unwrap_or(js::Node::Void)],
                )
            }
            Intrinsic::TryParseJson => {
                self.used_helpers.insert("__try_parse_json");
                js::Node::Call(
                    Box::new(js::Node::Local("__try_parse_json".to_string())),
                    vec![args.next().unwrap_or(js::Node::Void)],
                )
            }
            Intrinsic::ParseF64 => {
                self.used_helpers.insert("__parse_f64");
                js::Node::Call(
                    Box::new(js::Node::Local("__parse_f64".to_string())),
                    vec![args.next().unwrap_or(js::Node::Void)],
                )
            }
            Intrinsic::RandomInt => {
                self.used_helpers.insert("__random_int");
                js::Node::Call(
                    Box::new(js::Node::Local("__random_int".to_string())),
                    args.collect(),
                )
            }
            Intrinsic::RandomFloat => {
                self.used_helpers.insert("__random_float");
                js::Node::Call(
                    Box::new(js::Node::Local("__random_float".to_string())),
                    args.collect(),
                )
            }
            Intrinsic::Args => {
                self.used_helpers.insert("__args");
                js::Node::Call(Box::new(js::Node::Local("__args".to_string())), Vec::new())
            }
            Intrinsic::Env => {
                self.used_helpers.insert("__env");
                js::Node::Call(
                    Box::new(js::Node::Local("__env".to_string())),
                    vec![args.next().unwrap_or(js::Node::Void)],
                )
            }
            // `Shared::new(value)` -> a `{ v: value }` cell (a JS object, so
            // `__clone` shares it by reference rather than deep-copying).
            Intrinsic::SharedNew => {
                self.used_helpers.insert("__shared_new");
                js::Node::Call(
                    Box::new(js::Node::Local("__shared_new".to_string())),
                    vec![args.next().unwrap_or(js::Node::Void)],
                )
            }
            // `shared.clone()` -> the same cell (the receiver, unchanged).
            Intrinsic::SharedClone => args.next().unwrap_or(js::Node::Void),
            // `shared.read()` / `shared.write()` -> the cell's value, `self.v`.
            // `write` returns a view of the slot; the write-*through* (rebind vs
            // merge) is handled where the assignment is lowered.
            Intrinsic::SharedValue | Intrinsic::SharedWrite => js::Node::Property(
                Box::new(args.next().unwrap_or(js::Node::Void)),
                "v".to_string(),
            ),
            // `Set::new()` -> `new Set()` (no constructor args).
            Intrinsic::SetNew => {
                js::Node::Call(Box::new(js::Node::Local("new Set".to_string())), Vec::new())
            }
            Intrinsic::SetInsert => native_method(&mut args, "add"),
            Intrinsic::SetContains => native_method(&mut args, "has"),
            Intrinsic::SetRemove => native_method(&mut args, "delete"),
            Intrinsic::SetLen => js::Node::Property(
                Box::new(args.next().unwrap_or(js::Node::Void)),
                "size".to_string(),
            ),
            // `Map::new()` -> `new Map()` (no constructor args).
            Intrinsic::MapNew => {
                js::Node::Call(Box::new(js::Node::Local("new Map".to_string())), Vec::new())
            }
            Intrinsic::MapInsert => native_method(&mut args, "set"),
            Intrinsic::MapGet => {
                self.used_helpers.insert("__map_get");
                self.used_helpers.insert("__clone");
                js::Node::Call(
                    Box::new(js::Node::Local("__map_get".to_string())),
                    args.collect(),
                )
            }
            Intrinsic::MapContainsKey => native_method(&mut args, "has"),
            Intrinsic::MapRemove => native_method(&mut args, "delete"),
            Intrinsic::MapLen => js::Node::Property(
                Box::new(args.next().unwrap_or(js::Node::Void)),
                "size".to_string(),
            ),
            Intrinsic::MapKeys => {
                self.used_helpers.insert("__map_keys");
                self.used_helpers.insert("__clone");
                js::Node::Call(
                    Box::new(js::Node::Local("__map_keys".to_string())),
                    args.collect(),
                )
            }
            Intrinsic::MapValues => {
                self.used_helpers.insert("__map_values");
                self.used_helpers.insert("__clone");
                js::Node::Call(
                    Box::new(js::Node::Local("__map_values".to_string())),
                    args.collect(),
                )
            }
            Intrinsic::JsonField => {
                let receiver = args.next().unwrap_or(js::Node::Void);
                let key = args.next().unwrap_or(js::Node::Void);
                js::Node::PropertyIndex(Box::new(receiver), Box::new(key))
            }
            Intrinsic::JsonTag => {
                self.used_helpers.insert("__json_tag");
                js::Node::Call(
                    Box::new(js::Node::Local("__json_tag".to_string())),
                    args.collect(),
                )
            }
            // A parsed JSON array already is a JS array, so `elements` is the
            // receiver itself (typed as `List<JsonValue>`).
            Intrinsic::JsonElements => args.next().unwrap_or(js::Node::Void),
            // `self === null` — the `Option::None` discriminator.
            Intrinsic::JsonIsNull => js::Node::Binary(
                BinaryOp::Eq,
                Box::new(args.next().unwrap_or(js::Node::Void)),
                Box::new(js::Node::Null),
            ),
            // The normalized JSON kind string, for the decode type checks.
            Intrinsic::JsonKind => {
                self.used_helpers.insert("__json_kind");
                js::Node::Call(
                    Box::new(js::Node::Local("__json_kind".to_string())),
                    args.collect(),
                )
            }
            // The canonical key of a value (Hashable / value-keyed Map/Set).
            Intrinsic::CanonicalHash => {
                self.used_helpers.insert("__hash");
                js::Node::Call(
                    Box::new(js::Node::Local("__hash".to_string())),
                    args.collect(),
                )
            }
            // `Array.from(document.querySelectorAll(selector))` — the NodeList as a
            // real array, so `List` operations (`map`/`push`/…) behave.
            Intrinsic::QuerySelectorAll => {
                let query = js::Node::Call(
                    Box::new(js::Node::Local("document.querySelectorAll".to_string())),
                    vec![args.next().unwrap_or(js::Node::Void)],
                );
                js::Node::Call(
                    Box::new(js::Node::Local("Array.from".to_string())),
                    vec![query],
                )
            }
        }
    }

    /// `subject === <literal>` — the test a literal pattern compiles to.
    fn literal_equality(&mut self, literal_id: Id, subject: js::Node<'src>) -> js::Node<'src> {
        let mut throwaway = Vec::new();
        let literal = self
            .walk_entity(literal_id, &mut throwaway)
            .unwrap_or(js::Node::Void);
        js::Node::Binary(BinaryOp::Eq, Box::new(subject), Box::new(literal))
    }

    /// The capture variable ids a pattern binds, in order — so a guarded leg can
    /// clear their subject-slot aliases after the leg is compiled.
    fn pattern_capture_ids(pattern: &ExprPattern) -> Vec<Id> {
        let mut ids = Vec::new();
        fn collect(pattern: &ExprPattern, ids: &mut Vec<Id>) {
            match pattern {
                ExprPattern::Binding(capture_id) => ids.push(*capture_id),
                ExprPattern::Variant(_, _, payload) => {
                    for sub_pattern in payload {
                        collect(sub_pattern, ids);
                    }
                }
                ExprPattern::Tuple(elements) => {
                    for (sub_pattern, _width) in elements {
                        collect(sub_pattern, ids);
                    }
                }
                ExprPattern::Wildcard | ExprPattern::Literal(_) => {}
            }
        }
        collect(pattern, &mut ids);
        ids
    }

    fn compile_pattern(
        &mut self,
        pattern: &ExprPattern,
        subject: js::Node<'src>,
        conditions: &mut Vec<js::Node<'src>>,
        bindings: &mut Vec<js::Node<'src>>,
    ) {
        match pattern {
            ExprPattern::Wildcard => {}
            ExprPattern::Binding(capture_id) => {
                let name = self.ng.name_for(*capture_id);
                let mutable = self
                    .program
                    .variables
                    .get(capture_id)
                    .map(|variable| variable.mutable)
                    .unwrap_or(false);
                let variable = js::Variable {
                    name,
                    value: Box::new(subject),
                };
                bindings.push(if mutable {
                    js::Node::LetVariable(variable)
                } else {
                    js::Node::ConstVariable(variable)
                });
            }
            ExprPattern::Variant(enum_id, variant_index, payload) => {
                // `bool` and numeric (C-like) enums lower to native scalars, so
                // their variants test by value (`subject === true` / `=== -1`)
                // rather than by array discriminant slot.
                if let Some(test) = self.scalar_variant_test(*enum_id, *variant_index, &subject) {
                    conditions.push(test);
                    return;
                }
                conditions.push(js::Node::Binary(
                    BinaryOp::Eq,
                    Box::new(js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number("0".to_string(), None)),
                    )),
                    Box::new(js::Node::Number(variant_index.to_string(), None)),
                ));
                for (data_index, sub_pattern) in payload.iter().enumerate() {
                    // Variant data sits after the variant index.
                    let element = js::Node::PropertyIndex(
                        Box::new(subject.clone()),
                        Box::new(js::Node::Number((data_index + 1).to_string(), None)),
                    );
                    self.compile_pattern(sub_pattern, element, conditions, bindings);
                }
            }
            ExprPattern::Tuple(elements) => {
                // Tuples store flat: read each leaf at its flat offset, reslicing a
                // multi-slot (sub-tuple) capture.
                let mut leaves = Vec::new();
                Self::flatten_tuple_pattern(elements, &subject, 0, &mut leaves);
                for (sub_pattern, element) in leaves {
                    self.compile_pattern(sub_pattern, element, conditions, bindings);
                }
            }
            ExprPattern::Literal(literal_id) => {
                conditions.push(self.literal_equality(*literal_id, subject));
            }
        }
    }

    fn function(&mut self, function: &Function<'src>) -> js::Node<'src> {
        let name = self.ng.name_for(function.id);
        self.function_with_name(function, name)
    }

    fn function_with_name(&mut self, function: &Function<'src>, name: String) -> js::Node<'src> {
        let parameters = function
            .parameters
            .iter()
            .map(|parameter_id| js::Parameter {
                name: self.ng.name_for(*parameter_id),
            })
            .collect::<Vec<_>>();
        let mut body = self.walk_list(&function.body.0);
        if let Some(return_expr) = self.walk_entity(function.body.1, &mut body) {
            match return_expr {
                js::Node::Void => {}
                _ => {
                    body.push(js::Node::Return(Box::new(return_expr)));
                }
            }
        }
        js::Node::Function(js::Function {
            name,
            parameters,
            body,
            is_async: self.program.async_functions.contains(&function.id),
        })
    }

    /// Emits a concrete (non-generic) function once, keyed by its id. Any active
    /// substitution and self-type are cleared while walking it, since its body
    /// has no generic parameters of its own and is not a default being
    /// specialized.
    fn ensure_function_emitted(&mut self, function_id: Id) {
        if self.required_functions.contains_key(&function_id) {
            return;
        }
        // Already walking this body higher up the stack (a recursive call): the
        // call site just needs the name, so don't re-enter — otherwise a recursive
        // function would emit its body forever. The outer call records it below.
        if !self.emitting.insert(function_id) {
            return;
        }
        if let Some(function) = self.program.functions.get(&function_id) {
            let saved = std::mem::take(&mut self.current_substitution);
            let saved_self = self.current_self_type.take();
            let js_function = self.function(function);
            self.current_substitution = saved;
            self.current_self_type = saved_self;
            self.required_functions.insert(function_id, js_function);
        }
        self.emitting.remove(&function_id);
    }

    /// Re-dispatches a trait method call to the receiver's concrete `type_id`:
    /// resolves to the type's own impl member if it declares one, otherwise an
    /// inherited trait default specialized for the type (so the default's inner
    /// `self.method()` calls dispatch to this type too). The member may be an
    /// intrinsic or an `[extern]` external — which lower to a host form, not a
    /// call to an emitted function — so this returns a [`Dispatch`] describing
    /// how to emit it; `emit_dispatch` turns that into the actual call node. A
    /// generic dispatch resolving to an extern/intrinsic without this would mint
    /// a dangling name for a function that is never emitted.
    fn resolve_dispatch(&mut self, type_id: TypeId, member: &str) -> Option<Dispatch<'src>> {
        self.resolve_dispatch_with(type_id, member, &[], None)
    }

    /// `resolve_dispatch`, additionally binding the target method's OWN generics
    /// from `own_generic_values` (the call's bindings in declaration order —
    /// recorded against the trait member the analyzer saw, whose ids differ from
    /// each concrete impl's, so only positional values cross the re-dispatch),
    /// and — when the analyzer resolved the call through a trait — dispatching on
    /// THAT trait's surface (`preferred_trait`): its impl's override, else its
    /// default, never an inherent method that happens to share the name.
    fn resolve_dispatch_with(
        &mut self,
        type_id: TypeId,
        member: &str,
        own_generic_values: &[TypeId],
        preferred_trait: Option<Id>,
    ) -> Option<Dispatch<'src>> {
        let type_id = self.resolve_type_id(type_id);
        if let Some(trait_id) = preferred_trait {
            // Resolve strictly within the trait: the impl's override first...
            if let Some((member_id, impl_subject)) =
                self.resolve_member_on_trait_impl(type_id, trait_id, member)
            {
                return Some(self.dispatch_to_member(
                    member_id,
                    impl_subject,
                    type_id,
                    own_generic_values,
                ));
            }
            // ...else the trait's own default, specialized for this type.
            if let Some(default_id) = self.trait_default_member(trait_id, member) {
                let is_async = self.program.async_functions.contains(&default_id);
                return Some(Dispatch::Call(
                    self.emit_default_instance(default_id, type_id),
                    is_async,
                ));
            }
            // The preference didn't materialize (shouldn't happen for a call the
            // analyzer resolved) — fall through to the general lookup.
        }
        if let Some((member_id, impl_subject)) = self.resolve_member_on_type(type_id, member) {
            return Some(self.dispatch_to_member(
                member_id,
                impl_subject,
                type_id,
                own_generic_values,
            ));
        }
        let default_id = self.resolve_inherited_default(type_id, member)?;
        let is_async = self.program.async_functions.contains(&default_id);
        Some(Dispatch::Call(
            self.emit_default_instance(default_id, type_id),
            is_async,
        ))
    }

    /// Lowers a resolved member to its dispatch: an intrinsic, an extern, or an
    /// emitted (possibly monomorphized) instance. Binds the impl's generics from
    /// the concrete receiver type — so a method whose body uses the impl's type
    /// parameter (`T::from_json_value` inside `List<T>::from_json_value`)
    /// resolves it concretely even when reached as a *nested* dispatch — plus
    /// the method's OWN generics from the call's ordered values (without which
    /// the instance emitted with them unbound — the silent no-op through a
    /// bound).
    fn dispatch_to_member(
        &mut self,
        member_id: Id,
        impl_subject: TypeId,
        type_id: TypeId,
        own_generic_values: &[TypeId],
    ) -> Dispatch<'src> {
        if let Some(intrinsic) = self.program.intrinsics.get(&member_id).copied() {
            return Dispatch::Intrinsic(intrinsic);
        }
        if let Some(binding) = self
            .program
            .external_functions
            .get(&member_id)
            .and_then(|external| external.extern_binding.clone())
        {
            return Dispatch::Extern(member_id, binding);
        }
        let mut substitution = HashMap::new();
        self.bind_generics(impl_subject, type_id, &mut substitution);
        if !own_generic_values.is_empty() {
            if let Some(function) = self.program.functions.get(&member_id) {
                for (constraint_id, value) in function
                    .generic_parameter_constraint_ids
                    .iter()
                    .zip(own_generic_values.iter())
                {
                    substitution.insert(*constraint_id, *value);
                }
            }
        }
        let name = if substitution.is_empty() {
            self.ensure_function_emitted(member_id);
            self.ng.name_for(member_id)
        } else {
            self.emit_instance(member_id, &substitution)
        };
        let is_async = self.program.async_functions.contains(&member_id);
        Dispatch::Call(name, is_async)
    }

    /// A member provided by `type_id`'s impl OF `trait_id` specifically — the
    /// trait-scoped counterpart of `resolve_member_on_type`, immune to inherent
    /// name collisions.
    fn resolve_member_on_trait_impl(
        &self,
        type_id: TypeId,
        trait_id: Id,
        member: &str,
    ) -> Option<(Id, TypeId)> {
        let type_ = self.program.type_id_to_type_map.get(&type_id)?;
        self.program
            .implementations
            .iter()
            .filter(|implementation| {
                implementation.trait_ids.contains(&trait_id)
                    && self
                        .program
                        .type_id_to_type_map
                        .get(&implementation.subject)
                        .is_some_and(|subject| nominal_matches(subject, type_))
            })
            .find_map(|implementation| {
                implementation
                    .declarations
                    .get(member)
                    .map(|member_id| (*member_id, implementation.subject))
            })
    }

    /// Lowers a resolved [`Dispatch`] to its call node with `args` (the receiver
    /// is the first argument). An async member is awaited.
    fn emit_dispatch(
        &mut self,
        dispatch: Dispatch<'src>,
        args: Vec<js::Node<'src>>,
    ) -> js::Node<'src> {
        match dispatch {
            Dispatch::Intrinsic(intrinsic) => self.emit_intrinsic(intrinsic, args),
            Dispatch::Extern(member_id, binding) => {
                let call = self.emit_extern(member_id, binding, args);
                self.maybe_await(member_id, call)
            }
            Dispatch::Call(name, is_async) => {
                let call = js::Node::Call(Box::new(js::Node::Local(name)), args);
                if is_async {
                    js::Node::Await(Box::new(call))
                } else {
                    call
                }
            }
        }
    }

    /// Emits a trait default method specialized for a concrete type, keyed by
    /// (default, type) so each pairing is emitted once. While walking the body,
    /// `current_self_type` is the concrete type so its `self.method()` calls
    /// re-dispatch there.
    fn emit_default_instance(&mut self, default_id: Id, type_id: TypeId) -> String {
        let key = (default_id, self.type_key(type_id));
        if let Some(name) = self.default_instances.get(&key) {
            return name.clone();
        }
        let name = self.ng.next_name();
        self.default_instances.insert(key, name.clone());
        if let Some(function) = self.program.functions.get(&default_id) {
            let saved_self = std::mem::replace(&mut self.current_self_type, Some(type_id));
            let saved_substitution = std::mem::take(&mut self.current_substitution);
            let js_function = self.function_with_name(function, name.clone());
            self.current_self_type = saved_self;
            self.current_substitution = saved_substitution;
            self.monomorphized.push(js_function);
        }
        name
    }

    /// Resolves `member` as an inherited trait *default* on a concrete type — a
    /// member none of the type's impls declare, but a (super)trait it implements
    /// provides with a body. Mirrors the analyzer's Gap E resolution.
    fn resolve_inherited_default(&self, type_id: TypeId, member: &str) -> Option<Id> {
        let type_ = self.program.type_id_to_type_map.get(&type_id)?.clone();
        self.program
            .implementations
            .iter()
            .filter(|implementation| {
                // NOMINAL matching, like `resolve_member_on_type`: the impl
                // subject is written in its own generic terms (`Signal<T>`),
                // the receiver in concrete ones (`Signal<i32>`) — exact type
                // equality only ever matched non-generic subjects, silently
                // dropping inherited defaults on generic types (the emitted
                // call then bound to the trait's abstract member).
                self.program
                    .type_id_to_type_map
                    .get(&implementation.subject)
                    .is_some_and(|subject| nominal_matches(subject, &type_))
            })
            .flat_map(|implementation| implementation.trait_ids.iter().copied())
            .find_map(|trait_id| self.trait_default_member(trait_id, member))
    }

    /// Searches a trait and its supertraits for a default (bodied) member.
    fn trait_default_member(&self, trait_id: Id, member: &str) -> Option<Id> {
        let mut stack = vec![trait_id];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(trait_) = self.program.traits.get(&id) else {
                continue;
            };
            if let Some(&member_id) = trait_.declarations.get(member) {
                if self.function_has_body(member_id) {
                    return Some(member_id);
                }
            }
            for supertrait_type_id in &trait_.supertraits {
                if let Some(Type::Trait(super_id, _)) =
                    self.program.type_id_to_type_map.get(supertrait_type_id)
                {
                    stack.push(*super_id);
                }
            }
        }
        None
    }

    /// Whether `member_id` is a function with a source-provided body (a trait
    /// default, as opposed to a signature-only requirement).
    fn function_has_body(&self, member_id: Id) -> bool {
        match self.program.entity_map.get(&member_id) {
            Some(Expr::Function(function_id)) => self
                .program
                .functions
                .get(function_id)
                .map(|function| function.has_body)
                .unwrap_or(false),
            _ => false,
        }
    }

    /// The generic binding to monomorphize a call's callee with, drawn from
    /// whichever channel carries it — so the transformer reads a call's binding in
    /// one place and emits through the one [`Self::emit_instance`] path. In
    /// precedence order: a free generic call's positional type arguments
    /// (`id<i32>` -> `{T: i32}`); the receiver / own-generic substitution the
    /// analyzer recorded for a method or operator (`xs.sum()` on `List<i32>`); or,
    /// for a generic call nested in a monomorphized body whose arguments come only
    /// from the enclosing instantiation, the inherited slice of the active
    /// substitution. `None` means the callee is non-generic (or nothing binds it),
    /// so it is emitted as a plain function.
    fn call_substitution(
        &self,
        call_id: Id,
        target_id: Id,
        generic_argument_ids: &[TypeId],
    ) -> Option<HashMap<TypeId, TypeId>> {
        let function = self.program.functions.get(&target_id);
        let is_generic = function.is_some_and(|f| !f.generic_parameter_constraint_ids.is_empty());
        if is_generic && !generic_argument_ids.is_empty() {
            return Some(
                function
                    .unwrap()
                    .generic_parameter_constraint_ids
                    .iter()
                    .copied()
                    .zip(generic_argument_ids.iter().copied())
                    .collect(),
            );
        }
        if let Some(recorded) = self.program.method_call_substitution.get(&call_id) {
            return Some(recorded.clone());
        }
        let inherited = self.inherited_substitution(target_id);
        (!inherited.is_empty()).then_some(inherited)
    }

    /// Emits (or reuses) a monomorphized instance of `function_id` specialized by
    /// `substitution` (generic constraint id -> concrete type). This is the single
    /// monomorphization path for *every* generic instantiation — free function,
    /// impl/trait method, operator, nested call — so a binding flows through one
    /// place regardless of how it was recorded. Keyed by (function, bound types)
    /// so each instantiation is emitted once. While walking the body,
    /// `current_substitution` is the binding, so `T::default()` and `T`-typed
    /// values resolve concretely.
    fn emit_instance(&mut self, function_id: Id, substitution: &HashMap<TypeId, TypeId>) -> String {
        // Resolve each bound type under the active substitution (so a nested
        // instantiation composes) and order by constraint id for a stable key.
        let mut entries: Vec<(TypeId, TypeId)> = substitution
            .iter()
            .map(|(constraint_id, type_id)| (*constraint_id, self.resolve_type_id(*type_id)))
            .collect();
        entries.sort_by_key(|(constraint_id, _)| constraint_id.0);
        let key = (
            function_id,
            entries
                .iter()
                .map(|(_, type_id)| self.type_key(*type_id))
                .collect::<Vec<_>>(),
        );
        if let Some(name) = self.instances.get(&key) {
            return name.clone();
        }
        let substitution: HashMap<TypeId, TypeId> = entries.into_iter().collect();
        let name = self.ng.next_name();
        self.instances.insert(key, name.clone());
        if let Some(function) = self.program.functions.get(&function_id) {
            let saved = std::mem::replace(&mut self.current_substitution, substitution);
            let js_function = self.function_with_name(function, name.clone());
            self.current_substitution = saved;
            self.monomorphized.push(js_function);
        }
        name
    }

    /// The bindings the active substitution provides for the generics a callee's
    /// signature mentions — used to specialize a generic call whose type
    /// arguments come only from the enclosing monomorphization (so the analysis
    /// recorded no substitution of its own). Empty when nothing applies, so the
    /// caller falls back to a plain (generic) emission.
    fn inherited_substitution(&self, target_id: Id) -> HashMap<TypeId, TypeId> {
        if self.current_substitution.is_empty() {
            return HashMap::new();
        }
        let Some(function) = self.program.functions.get(&target_id) else {
            return HashMap::new();
        };
        let mut generics = Vec::new();
        for parameter_id in &function.parameters {
            if let Some(parameter) = self.program.parameters.get(parameter_id) {
                self.collect_type_generics(parameter.type_id, 0, &mut generics);
            }
        }
        if let Some(return_type_id) = function.return_type_id {
            self.collect_type_generics(return_type_id, 0, &mut generics);
        }
        generics
            .into_iter()
            .filter_map(|constraint_id| {
                self.current_substitution
                    .get(&constraint_id)
                    .map(|type_id| (constraint_id, *type_id))
            })
            .collect()
    }

    /// Collects the `Generic` constraint ids a type's structure mentions (its own
    /// id, or those nested in a struct/enum/tuple/closure's arguments).
    fn collect_type_generics(&self, type_id: TypeId, depth: usize, out: &mut Vec<TypeId>) {
        if depth > 24 {
            return;
        }
        match self.program.type_id_to_type_map.get(&type_id) {
            Some(Type::Generic(constraint_id)) => {
                if !out.contains(constraint_id) {
                    out.push(*constraint_id);
                }
            }
            Some(
                Type::Struct(_, arguments) | Type::Enum(_, arguments) | Type::Tuple(arguments),
            ) => {
                for argument in arguments.clone() {
                    self.collect_type_generics(argument, depth + 1, out);
                }
            }
            Some(Type::Closure(parameters, return_type_id)) => {
                let parameters = parameters.clone();
                let return_type_id = *return_type_id;
                for parameter in parameters {
                    self.collect_type_generics(parameter, depth + 1, out);
                }
                self.collect_type_generics(return_type_id, depth + 1, out);
            }
            _ => {}
        }
    }

    /// Resolves a type id to its concrete form under the active substitution,
    /// following generic parameters to the type they're currently bound to.
    /// The resolved type id of an expression, used for tuple flat-layout
    /// decisions. Falls back through a binding reference to the binding's type
    /// (a bare `Expr::Local`/`Parameter` use carries no type on its own id).
    fn expr_type_id(&self, expr_id: Id) -> Option<TypeId> {
        if let Some(type_id) = self.program.expr_type_ids.get(&expr_id) {
            return Some(*type_id);
        }
        match self.program.entity_map.get(&expr_id)? {
            Expr::Local(binding) | Expr::Variable(binding) => {
                self.program.variables.get(binding).map(|v| v.type_id)
            }
            Expr::Parameter(binding) => self.program.parameters.get(binding).map(|p| p.type_id),
            _ => None,
        }
    }

    /// Whether an expression's (monomorphized) type is a tuple — its value is a
    /// flat array whose slots splice into a constructed tuple. A tuple literal is
    /// recognized structurally (its own id carries no stored type); anything else
    /// is decided by its resolved type.
    fn is_tuple_typed(&self, expr_id: Id) -> bool {
        if matches!(self.program.entity_map.get(&expr_id), Some(Expr::Tuple(_))) {
            return true;
        }
        self.expr_type_id(expr_id)
            .map(|type_id| self.resolve_type_id(type_id))
            .and_then(|type_id| self.program.type_id_to_type_map.get(&type_id))
            .is_some_and(|type_| matches!(type_, Type::Tuple(_)))
    }

    /// Whether an expression's (monomorphized) type is the built-in `Set` — a
    /// vilan struct wrapping a `NativeMap` (I1). Its elements are the backing
    /// map's stored originals, so `for x in set` iterates `set[0].values()`.
    fn is_set_typed(&self, expr_id: Id) -> bool {
        self.expr_type_id(expr_id)
            .map(|type_id| self.resolve_type_id(type_id))
            .and_then(|type_id| self.program.type_id_to_type_map.get(&type_id))
            .is_some_and(|type_| match type_ {
                Type::Struct(id, _) => self
                    .program
                    .structs
                    .get(id)
                    .is_some_and(|struct_| struct_.name == "Set"),
                _ => false,
            })
    }

    fn resolve_type_id(&self, type_id: TypeId) -> TypeId {
        let Some(_guard) = crate::util::RecursionGuard::enter() else {
            return type_id;
        };
        match self.program.type_id_to_type_map.get(&type_id) {
            Some(Type::Generic(constraint_id)) => {
                match self.current_substitution.get(constraint_id) {
                    // Guard a self-mapping (`T -> T`): the substitution binds the
                    // generic to itself (which reconciling an impl's own parameter
                    // records), so following it would loop forever — leave it abstract.
                    Some(bound)
                        if !matches!(
                            self.program.type_id_to_type_map.get(bound),
                            Some(Type::Generic(c)) if c == constraint_id
                        ) =>
                    {
                        self.resolve_type_id(*bound)
                    }
                    _ => type_id,
                }
            }
            _ => type_id,
        }
    }

    /// A type whose `==`/`!=` compares by value in native JS — the scalar
    /// primitives (`i32`/…/`str`), `bool`, and numeric (C-like) enums, all lowered
    /// to JS numbers/strings/booleans. A generic `==` monomorphized to one of these
    /// stays native rather than dispatching to a `PartialEq` impl (which for a
    /// primitive is native `===` anyway), keeping codegen identical to a direct `==`.
    fn compares_natively(&self, type_id: TypeId) -> bool {
        match self.program.type_id_to_type_map.get(&type_id) {
            Some(Type::Struct(id, _)) => self.program.structs.get(id).is_some_and(|struct_| {
                matches!(struct_.name, "i32" | "u32" | "f64" | "BigInt" | "str")
            }),
            Some(Type::Enum(id, _)) => {
                Some(*id) == self.program.bool_enum_id
                    || self
                        .program
                        .enums
                        .get(id)
                        .is_some_and(|enum_| enum_.is_numeric)
            }
            _ => false,
        }
    }

    /// A stable key identifying a concrete type, used to deduplicate instances.
    fn type_key(&self, type_id: TypeId) -> String {
        match self.program.type_id_to_type_map.get(&type_id) {
            Some(type_) => format!("{:?}", type_),
            None => format!("?{}", type_id.0),
        }
    }

    /// Finds the function implementing `member` for a concrete type, searching
    /// the implementations whose subject matches that type.
    /// Resolves `member` on a concrete type to its impl method, returning the
    /// member id *and the impl's subject* (in the impl's own generic terms, e.g.
    /// `List<Generic(T)>`) so the caller can bind the impl's generics from the
    /// concrete type's arguments.
    fn resolve_member_on_type(&self, type_id: TypeId, member: &str) -> Option<(Id, TypeId)> {
        let type_ = self.program.type_id_to_type_map.get(&type_id)?;
        match type_ {
            Type::Struct(_, _) | Type::Enum(_, _) => self
                .program
                .implementations
                .iter()
                .filter(|implementation| {
                    self.program
                        .type_id_to_type_map
                        .get(&implementation.subject)
                        .is_some_and(|subject| nominal_matches(subject, type_))
                })
                .find_map(|implementation| {
                    implementation
                        .declarations
                        .get(member)
                        .map(|member_id| (*member_id, implementation.subject))
                }),
            _ => None,
        }
    }

    /// Binds the generic parameters in `pattern` (an impl subject in its own
    /// generic terms, `List<Generic(T)>`) from the matching positions of the
    /// concrete `type_id` (`List<i32>`), accumulating `{T -> i32}`. Recurses
    /// through nominal arguments, tuples, and closures so a nested parameter
    /// (`List<List<T>>` -> `T = i32`) is reached.
    fn bind_generics(&self, pattern: TypeId, type_id: TypeId, out: &mut HashMap<TypeId, TypeId>) {
        let Some(pattern_type) = self.program.type_id_to_type_map.get(&pattern).cloned() else {
            return;
        };
        if let Type::Generic(constraint_id) = pattern_type {
            out.insert(constraint_id, type_id);
            return;
        }
        let Some(concrete_type) = self.program.type_id_to_type_map.get(&type_id).cloned() else {
            return;
        };
        let zip_args = |out: &mut HashMap<TypeId, TypeId>,
                        pattern_args: &[TypeId],
                        concrete_args: &[TypeId],
                        this: &Self| {
            for (pattern_arg, concrete_arg) in pattern_args.iter().zip(concrete_args.iter()) {
                this.bind_generics(*pattern_arg, *concrete_arg, out);
            }
        };
        match (pattern_type, concrete_type) {
            (Type::Struct(a, pattern_args), Type::Struct(b, concrete_args)) if a == b => {
                zip_args(out, &pattern_args, &concrete_args, self);
            }
            (Type::Enum(a, pattern_args), Type::Enum(b, concrete_args)) if a == b => {
                zip_args(out, &pattern_args, &concrete_args, self);
            }
            (Type::Tuple(pattern_args), Type::Tuple(concrete_args)) => {
                zip_args(out, &pattern_args, &concrete_args, self);
            }
            (
                Type::Closure(pattern_params, pattern_ret),
                Type::Closure(concrete_params, concrete_ret),
            ) => {
                zip_args(out, &pattern_params, &concrete_params, self);
                self.bind_generics(pattern_ret, concrete_ret, out);
            }
            _ => {}
        }
    }
}

#[derive(Clone)]
struct Formatter {
    line_break: &'static str,
    indentation: &'static str,
    space: &'static str,
    array_surround: &'static str,
    // object_surround: &'static str,
}

impl Formatter {
    /// Builds the whitespace style from the two formatting options: `indent` gives
    /// line breaks + leading indentation, `spaces` gives inter-token padding. They
    /// are independent — `indent && !spaces` is multi-line but tight, for example.
    fn from_options(indent: bool, spaces: bool) -> Self {
        Self {
            line_break: if indent { "\n" } else { "" },
            indentation: if indent { "\t" } else { "" },
            space: if spaces { " " } else { "" },
            array_surround: if spaces { " " } else { "" },
        }
    }

    fn file(&self, list: &Vec<js::Node>) -> String {
        self.sequence(list, ";", 0)
    }

    /// Renders a sequence of statements, one per line, each indented to `level`.
    /// The per-statement indent lives here (not in `node`) so `node` can render a
    /// sub-expression inline — without a leading indent — while still passing the
    /// current `level` down, so a block nested inside an expression (a closure
    /// argument, a function-valued binding) indents to its true depth.
    fn sequence(&self, list: &[js::Node], terminator: &'static str, level: usize) -> String {
        let indent = self.indentation.repeat(level);
        list.iter()
            .map(|node| format!("{}{}", indent, self.node(node, terminator, level)))
            .collect::<Vec<_>>()
            .join(self.line_break)
    }

    /// A binary operator's JavaScript binding precedence (higher binds tighter),
    /// used to parenthesize operands. Note this is JS's C-style order — distinct
    /// from vilan's source precedence (bitwise binds tighter than comparison in
    /// vilan, looser in JS), which is exactly why emission must parenthesize by
    /// THIS table, not the parser's.
    fn js_binary_precedence(op: BinaryOp) -> u8 {
        match op {
            BinaryOp::Or => 0,
            BinaryOp::And => 1,
            // JS's C-style order: the bitwise ops bind LOOSER than comparison —
            // the opposite of vilan's source precedence, so a vilan
            // `(a & b) == c` tree emits with parentheses.
            BinaryOp::BitOr => 2,
            BinaryOp::BitXor => 3,
            BinaryOp::BitAnd => 4,
            BinaryOp::Eq | BinaryOp::NotEq => 5,
            BinaryOp::Lt | BinaryOp::Gt | BinaryOp::LtEq | BinaryOp::GtEq => 6,
            BinaryOp::Shl | BinaryOp::Shr | BinaryOp::UShr => 7,
            BinaryOp::Add | BinaryOp::Sub => 8,
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => 9,
        }
    }

    /// Renders a binary node's operand, parenthesizing when its own binding is
    /// too loose to survive unwrapped: a nested binary whose precedence fails
    /// `keeps(child)`, or an assignment (which JS parses greedily as an
    /// expression). Atoms, calls, and property accesses bind tighter than any
    /// binary operator and pass through bare.
    fn operand(&self, node: &js::Node, level: usize, keeps: impl Fn(u8) -> bool) -> String {
        let rendered = self.node(node, "", level);
        let wrap = match node {
            js::Node::Binary(op, _, _) => !keeps(Self::js_binary_precedence(*op)),
            js::Node::Assignment(_, _) => true,
            _ => false,
        };
        if wrap {
            format!("({rendered})")
        } else {
            rendered
        }
    }

    /// Renders one JavaScript node at block-nesting `level` (used to indent the
    /// bodies of any nested blocks). It emits no leading indent of its own — a
    /// statement's indent is added by `sequence`, an expression is rendered inline
    /// — and passes `level` down to its sub-expressions, so a block nested inside
    /// an expression indents to its true depth.
    fn node(&self, node: &js::Node, terminator: &'static str, level: usize) -> String {
        match node {
            js::Node::Void => format!("undefined{}", terminator),
            js::Node::Null => format!("null{}", terminator),
            js::Node::String(x) => format!("\"{}\"{}", x.escape_default(), terminator),
            js::Node::Number(whole, fraction) => format!(
                "{}{}{}",
                whole,
                fraction
                    .clone()
                    .map(|x| format!(".{x}"))
                    .unwrap_or("".to_string()),
                terminator
            ),
            js::Node::Bool(x) => format!("{}{}", x, terminator),
            js::Node::Array(items) => {
                let s_items = items
                    .iter()
                    .map(|x| self.node(x, "", level))
                    .collect::<Vec<_>>()
                    .join(format!(",{}", self.space).as_str());
                format!(
                    "[{}{}{}]{}",
                    self.array_surround, s_items, self.array_surround, terminator
                )
            }
            js::Node::Spread(operand) => {
                format!("...{}{}", self.node(operand, "", level), terminator)
            }
            js::Node::Function(function) => {
                let name = function.name.as_str();
                let parameters = function
                    .parameters
                    .iter()
                    .map(|x| x.name.as_str())
                    .collect::<Vec<_>>()
                    .join(format!(",{}", self.space).as_str());
                let body = self.sequence(&function.body, ";", level + 1);
                format!(
                    "{}function {}({}){}{{{}{}{}{}}}{}",
                    if function.is_async { "async " } else { "" },
                    name,
                    parameters,
                    self.space,
                    self.line_break,
                    body,
                    self.line_break,
                    self.indentation.repeat(level),
                    match terminator {
                        ";" => "",
                        x => x,
                    }
                )
            }
            js::Node::Local(name) => format!("{}{}", name, terminator),
            js::Node::Assignment(subject, value) => format!(
                "{}{}={}{}{}",
                self.node(subject, "", level),
                self.space,
                self.space,
                self.node(value, "", level),
                terminator
            ),
            js::Node::Return(value) => match &**value {
                js::Node::Void => format!("return{}", terminator),
                x => format!("return {}{}", self.node(x, "", level), terminator),
            },
            js::Node::Throw(value) => {
                format!("throw {}{}", self.node(value, "", level), terminator)
            }
            js::Node::Call(subject, args) => {
                let s_subject = self.node(subject, "", level);
                // A closure called directly must be parenthesised: `(() => …)()`.
                let s_subject = if matches!(&**subject, js::Node::Closure(_)) {
                    format!("({s_subject})")
                } else {
                    s_subject
                };
                let s_args = args
                    .iter()
                    .map(|x| self.node(x, "", level))
                    .collect::<Vec<_>>()
                    .join(format!(",{}", self.space).as_str());
                format!("{}({}){}", s_subject, s_args, terminator)
            }
            js::Node::Binary(op, lhs, rhs) => {
                let s_op = match op {
                    BinaryOp::Add => "+",
                    BinaryOp::Sub => "-",
                    BinaryOp::Mul => "*",
                    BinaryOp::Div => "/",
                    BinaryOp::Rem => "%",
                    BinaryOp::Shl => "<<",
                    BinaryOp::Shr => ">>",
                    BinaryOp::UShr => ">>>",
                    BinaryOp::BitAnd => "&",
                    BinaryOp::BitXor => "^",
                    BinaryOp::BitOr => "|",
                    BinaryOp::Eq => "===",
                    BinaryOp::NotEq => "!==",
                    BinaryOp::Lt => "<",
                    BinaryOp::Gt => ">",
                    BinaryOp::LtEq => "<=",
                    BinaryOp::GtEq => ">=",
                    BinaryOp::And => "&&",
                    BinaryOp::Or => "||",
                };
                // Operands are parenthesized by JS precedence, or grouping is
                // lost — `(1 + 2) * 3` must not print as `1 + 2 * 3`. The left
                // operand needs parens when it binds looser than this node; the
                // right also at EQUAL precedence (`-`/`/` are non-associative,
                // and `+` mixes strings and numbers, so `1 + (2 + "x")` differs
                // from `1 + 2 + "x"`).
                let parent = Self::js_binary_precedence(*op);
                let s_lhs = self.operand(lhs, level, |child| child >= parent);
                let s_rhs = self.operand(rhs, level, |child| child > parent);
                format!(
                    "{}{}{}{}{}{}",
                    s_lhs, self.space, s_op, self.space, s_rhs, terminator
                )
            }
            js::Node::Unary(operator, operand) => {
                // Parenthesise the operand so precedence is preserved — e.g.
                // `!(a < b)` must not render as `!a < b`.
                format!(
                    "{}({}){}",
                    operator,
                    self.node(operand, "", level),
                    terminator
                )
            }
            js::Node::LetVariable(variable) => {
                let value = self.node(&variable.value, "", level);
                format!(
                    "let {}{}={}{}{}",
                    variable.name, self.space, self.space, value, terminator
                )
            }
            js::Node::ConstVariable(variable) => {
                let value = self.node(&variable.value, "", level);
                format!(
                    "const {}{}={}{}{}",
                    variable.name, self.space, self.space, value, terminator
                )
            }
            js::Node::Property(subject, member) => {
                let s_subject = self.node(subject, "", level);
                format!("{}.{}{}", s_subject, member, terminator)
            }
            js::Node::PropertyIndex(subject, member) => {
                let s_subject = self.node(subject, "", level);
                let s_member = self.node(member, "", level);
                format!("{}[{}]{}", s_subject, s_member, terminator)
            }
            js::Node::If(branch) => {
                fn walk_branch(
                    f: &Formatter,
                    branch: &js::IfBranch,
                    level: usize,
                    else_depth: u32,
                ) -> String {
                    match branch {
                        js::IfBranch::If(condition, body, else_) => {
                            let s_prefix = if else_depth > 0 { "else " } else { "" };
                            let s_condition = f.node(condition, "", level);
                            let s_body = f.sequence(body, ";", level + 1);
                            let s_else = else_
                                .as_ref()
                                .map(|x| {
                                    format!(
                                        "{}{}",
                                        f.space,
                                        walk_branch(f, x, level, else_depth + 1)
                                    )
                                })
                                .unwrap_or("".to_string());
                            format!(
                                "{}if{}({}){}{{{}{}{}{}}}{}",
                                s_prefix,
                                f.space,
                                s_condition,
                                f.space,
                                f.line_break,
                                s_body,
                                f.line_break,
                                f.indentation.repeat(level),
                                s_else
                            )
                        }
                        js::IfBranch::Else(body) => {
                            let s_body = f.sequence(body, ";", level + 1);
                            format!(
                                "else{}{{{}{}{}{}}}",
                                f.space,
                                f.line_break,
                                s_body,
                                f.line_break,
                                f.indentation.repeat(level)
                            )
                        }
                    }
                }
                walk_branch(self, branch, level, 0)
            }
            js::Node::While(condition, body) => {
                let s_condition = self.node(condition, "", level);
                let s_body = self.sequence(body, ";", level + 1);
                format!(
                    "while{}({}){}{{{}{}{}{}}}",
                    self.space,
                    s_condition,
                    self.space,
                    self.line_break,
                    s_body,
                    self.line_break,
                    self.indentation.repeat(level),
                )
            }
            js::Node::ForOf(binding, iterable, body) => {
                let s_iterable = self.node(iterable, "", level);
                let s_body = self.sequence(body, ";", level + 1);
                format!(
                    "for{}(const {} of {}){}{{{}{}{}{}}}",
                    self.space,
                    binding,
                    s_iterable,
                    self.space,
                    self.line_break,
                    s_body,
                    self.line_break,
                    self.indentation.repeat(level),
                )
            }
            js::Node::Break => format!("break{}", terminator),
            js::Node::Continue => format!("continue{}", terminator),
            js::Node::Closure(closure) => {
                let s_parameters = closure
                    .parameters
                    .iter()
                    .map(|x| x.name.as_str())
                    .collect::<Vec<_>>()
                    .join(format!(",{}", self.space).as_str());
                let s_body = self.sequence(&closure.body, ";", level + 1);
                format!(
                    "{}({}){}=>{}{{{}{}{}{}}}{}",
                    if closure.is_async { "async " } else { "" },
                    s_parameters,
                    self.space,
                    self.space,
                    self.line_break,
                    s_body,
                    self.line_break,
                    self.indentation.repeat(level),
                    terminator
                )
            }
            js::Node::Await(operand) => {
                // Parenthesise so `await` doesn't bind too loosely (e.g.
                // `await (a + b)`), mirroring the unary `!` rendering.
                format!("await ({}){}", self.node(operand, "", level), terminator)
            }
        }
    }
}

/// Serializes a const result in place of its expression (const-eval.md §1),
/// producing the same runtime shapes emitted code builds itself: structs and
/// enums are already positional arrays at this level, so `ConstValue::Array`
/// covers them.
fn const_value_to_js<'src>(value: &ConstValue) -> js::Node<'src> {
    match value {
        ConstValue::Undefined => js::Node::Void,
        ConstValue::Null => js::Node::Null,
        ConstValue::Bool(value) => js::Node::Bool(*value),
        ConstValue::Number(n) => {
            if n.is_nan() {
                js::Node::Local("NaN".to_string())
            } else if n.is_infinite() {
                js::Node::Local(if *n > 0.0 { "Infinity" } else { "-Infinity" }.to_string())
            } else if *n == 0.0 && n.is_sign_negative() {
                // `js_number_to_string` collapses -0 to "0" (string coercion
                // semantics); the LITERAL must keep the sign.
                js::Node::Number("-0".to_string(), None)
            } else {
                js::Node::Number(crate::interpreter::js_number_to_string(*n), None)
            }
        }
        ConstValue::BigInt(n) => js::Node::Number(format!("{n}n"), None),
        ConstValue::Str(s) => js::Node::String(Cow::Owned(s.clone())),
        ConstValue::Array(items) => js::Node::Array(items.iter().map(const_value_to_js).collect()),
        ConstValue::Set(items) => js::Node::Call(
            Box::new(js::Node::Local("new Set".to_string())),
            vec![js::Node::Array(
                items.iter().map(const_value_to_js).collect(),
            )],
        ),
        ConstValue::Map(entries) => js::Node::Call(
            Box::new(js::Node::Local("new Map".to_string())),
            vec![js::Node::Array(
                entries
                    .iter()
                    .map(|(key, value)| {
                        js::Node::Array(vec![const_value_to_js(key), const_value_to_js(value)])
                    })
                    .collect(),
            )],
        ),
    }
}

/// Builds the mini-program that evaluates one `const` expression: the
/// functions it (transitively) requires, declarations for the bindings it
/// reads — already-computed const values as literals, literal initializers
/// walked — and a final `const __const_result = <expr>;`. Returns the program
/// plus any referenced bindings that are NOT compile-time-known; the caller
/// turns those into diagnostics (free variables inside the expression itself
/// are pre-checked with precise spans — this net catches what called
/// functions reach). Skips `rename_for_scopes`: names stay as minted, and
/// `__const_result` must survive for the evaluator to read.
pub fn transform_const_program<'src>(
    program: &'src Program<'src>,
    options: &BuildOptions,
    expr_id: Id,
    external_bindings: &HashSet<Id>,
    const_values: &HashMap<Id, crate::interpreter::ConstValue>,
) -> (JsProgram<'src>, Vec<Id>) {
    let mut transformer = Transformer::new(program, options);

    // The bindings that may need a prelude declaration: the expression's own
    // free locals (checked by the caller) and module-level bindings reached
    // through called functions. Everything else referenced is declared inside
    // the emitted code itself (function-body and block locals).
    let external: HashSet<Id> = program
        .module_level_bindings()
        .into_iter()
        .chain(external_bindings.iter().copied())
        .collect();

    let mut body = Vec::new();
    let result = transformer
        .walk_entity(expr_id, &mut body)
        .unwrap_or(js::Node::Void);

    // Emitting a binding's initializer can reference more bindings (and
    // require more functions) — iterate to a fixpoint.
    let mut declared: HashSet<Id> = HashSet::new();
    let mut unresolved: Vec<Id> = Vec::new();
    let mut prelude: Vec<js::Node<'src>> = Vec::new();
    loop {
        let pending: Vec<Id> = transformer
            .referenced_globals
            .iter()
            .copied()
            .filter(|id| external.contains(id) && !declared.contains(id))
            .collect();
        if pending.is_empty() {
            break;
        }
        for binding in pending {
            declared.insert(binding);
            // Non-variable references (functions, struct names) emit through
            // their own channels; only value bindings need declarations.
            let Some(variable) = program.variables.get(&binding) else {
                continue;
            };
            let name = transformer.ng.name_for(binding);
            // A const-initialized binding's computed value, keyed by its
            // INITIAL expression id (how `const_eval` stores results).
            if let Some(value) = variable
                .initial
                .and_then(|initial| const_values.get(&initial))
            {
                prelude.push(js::Node::ConstVariable(js::Variable {
                    name,
                    value: Box::new(const_value_to_js(value)),
                }));
                continue;
            }
            let initial = variable.initial;
            let literal_initial = initial
                .and_then(|initial| program.entity_map.get(&initial))
                .map(|entity| {
                    matches!(
                        entity,
                        Expr::String(_)
                            | Expr::MultilineString(_)
                            | Expr::Number(..)
                            | Expr::Bool(_)
                            | Expr::Null
                    )
                })
                .unwrap_or(false);
            if literal_initial && !variable.mutable {
                let value = transformer
                    .walk_entity(initial.unwrap(), &mut prelude)
                    .unwrap_or(js::Node::Void);
                prelude.push(js::Node::ConstVariable(js::Variable {
                    name,
                    value: Box::new(value),
                }));
            } else {
                unresolved.push(binding);
            }
        }
    }

    let mut t_functions: Vec<_> = transformer.required_functions.into_iter().collect();
    t_functions.sort_by(|a, b| (a.0.0).cmp(&b.0.0));
    let t_instances = transformer.monomorphized.into_iter();

    let imports = transformer
        .used_imports
        .iter()
        .map(|(module, symbols)| {
            let names = symbols.iter().cloned().collect::<Vec<_>>().join(", ");
            format!("import {{ {} }} from \"{}\";", names, module)
        })
        .collect::<Vec<_>>();
    if !program.clone_sites.is_empty() {
        transformer.used_helpers.insert("__clone");
    }
    let helpers = transformer.used_helpers.into_iter().collect::<Vec<_>>();

    body.push(js::Node::ConstVariable(js::Variable {
        name: "__const_result".to_string(),
        value: Box::new(result),
    }));
    let nodes = t_functions
        .into_iter()
        .map(|x| x.1)
        .chain(t_instances)
        .chain(prelude)
        .chain(body)
        .collect::<Vec<_>>();

    (
        JsProgram {
            imports,
            helpers,
            nodes,
        },
        unresolved,
    )
}

pub mod js {
    use crate::node::BinaryOp;
    use std::borrow::Cow;

    #[derive(Clone, Debug)]
    pub enum Node<'src> {
        Array(Vec<Self>),
        // `...<operand>` — array spread, used to splice a tuple-typed element's
        // (already flat) slots into a constructed tuple.
        Spread(Box<Self>),
        Assignment(Box<Self>, Box<Self>),
        // `await <operand>`.
        Await(Box<Self>),
        Binary(BinaryOp, Box<Self>, Box<Self>),
        Unary(char, Box<Self>),
        Bool(bool),
        Break,
        Call(Box<Self>, Vec<Self>),
        Closure(Closure<'src>),
        ConstVariable(Variable<'src>),
        Continue,
        Function(Function<'src>),
        If(IfBranch<'src>),
        While(Box<Self>, Vec<Self>),
        // `for (const <binding> of <iterable>) { <body> }`. The binding name is
        // `_` for a discarded element.
        ForOf(String, Box<Self>, Vec<Self>),
        LetVariable(Variable<'src>),
        Local(String), // TODO: Consider extracting identifiers into a separate lookup table for late identifier substitution.
        Null,
        Number(String, Option<String>),
        // Object(Vec<(&'src str, Self)>),
        Property(Box<Self>, String),
        PropertyIndex(Box<Self>, Box<Self>),
        Return(Box<Self>),
        String(Cow<'src, str>),
        Throw(Box<Self>),
        Void,
    }

    #[derive(Clone, Debug)]
    pub enum IfBranch<'src> {
        If(Box<Node<'src>>, Vec<Node<'src>>, Option<Box<Self>>),
        Else(Vec<Node<'src>>),
    }

    #[derive(Clone, Debug)]
    pub struct Function<'src> {
        pub name: String,
        pub parameters: Vec<Parameter>,
        pub body: Vec<Node<'src>>,
        pub is_async: bool,
    }

    #[derive(Clone, Debug)]
    pub struct Parameter {
        pub name: String,
    }

    #[derive(Clone, Debug)]
    pub struct Variable<'src> {
        pub name: String,
        pub value: Box<Node<'src>>,
    }

    #[derive(Clone, Debug)]
    pub struct Closure<'src> {
        pub parameters: Vec<Parameter>,
        pub body: Vec<Node<'src>>,
        pub is_async: bool,
    }
}

/// JavaScript reserved words, the globals the runtime/codegen reference, and the
/// `__`-prefixed runtime helpers — names a readable identifier must avoid. Per-
/// program `[extern]` symbols are added on top (see `collect_reserved_names`).
const RESERVED_NAMES: &[&str] = &[
    // Reserved words (a binding can't use these).
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "let",
    "static",
    "enum",
    "await",
    "async",
    "implements",
    "interface",
    "package",
    "private",
    "protected",
    "public",
    // Globals the runtime helpers / codegen reference as free identifiers.
    "console",
    "process",
    "Math",
    "JSON",
    "Number",
    "BigInt",
    "Boolean",
    "String",
    "Array",
    "Object",
    "Set",
    "Map",
    "Promise",
    "Symbol",
    "Date",
    "Error",
    "RegExp",
    "undefined",
    "NaN",
    "Infinity",
    "globalThis",
    "require",
    "module",
    "exports",
    "structuredClone",
    "setTimeout",
    "setInterval",
    "fetch",
    "document",
    "window",
    "Response",
    "Request",
    // Runtime helpers (emitted as `function __clone(..)`, etc.).
    "__clone",
    "__scan",
    "__parse_i32",
    "__parse_f64",
    "__random_int",
    "__random_float",
    "__args",
    "__env",
    "__shared_new",
    "__list_get",
    "__list_pop",
    "__map_get",
    "__map_keys",
    "__map_values",
];

/// The free identifiers a program's `[extern]`s introduce — an imported symbol
/// (`createServer`) or a global root (`console` from `console.log`) — which a
/// readable name must not shadow.
fn collect_reserved_names(program: &Program) -> HashSet<String> {
    let mut reserved: HashSet<String> =
        RESERVED_NAMES.iter().map(|name| name.to_string()).collect();
    for external in program.external_functions.values() {
        if let Some(ExternBinding::Function { symbol, .. }) = &external.extern_binding {
            if let Some(root) = symbol.split('.').next() {
                reserved.insert(root.to_string());
            }
        }
    }
    reserved
}

/// Turns a source name into a valid JS identifier — Vilan identifiers already are
/// (besides reserved words, handled at disambiguation), so this only guards the
/// degenerate cases.
fn sanitize_identifier(name: &str) -> String {
    let mut result: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if result.is_empty() || result.starts_with(|c: char| c.is_ascii_digit()) {
        result.insert(0, '_');
    }
    result
}

/// How generated identifiers are named.
enum NameStyle {
    /// After the source (`greet`), disambiguated on collision — most debuggable.
    Readable,
    /// Obfuscated short name with a source annotation (`a/*greet*/`).
    Annotated,
    /// Obfuscated short name only (`a`).
    Plain,
}

struct NameGenerator {
    chars: Vec<char>,
    counter: u64,
    names: HashMap<Id, String>,
    /// Source names by id (functions, variables, parameters) — empty for `Plain`.
    source_names: HashMap<Id, String>,
    style: NameStyle,
    /// Names already in use (readable mode): the reserved set plus every readable
    /// name assigned so far, so the next is disambiguated against them.
    taken: HashSet<String>,
}

impl NameGenerator {
    fn new(style: NameStyle, source_names: HashMap<Id, String>, reserved: HashSet<String>) -> Self {
        Self {
            chars: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
                .chars()
                .collect(),
            counter: 0,
            names: HashMap::new(),
            source_names,
            style,
            taken: reserved,
        }
    }

    fn name_for(&mut self, id: Id) -> String {
        if let Some(name) = self.names.get(&id) {
            return name.clone();
        }
        let name = match self.style {
            // Name after the source; an entity with no source name (an anonymous
            // temp) gets a `$`-prefixed fresh name, which no source name can be.
            NameStyle::Readable => match self.source_names.get(&id).cloned() {
                Some(source) => self.unique_readable(&source),
                None => self.next_name(),
            },
            NameStyle::Annotated => match self.source_names.get(&id).cloned() {
                Some(source) => format!("{}/*{}*/", self.next_name(), source),
                None => self.next_name(),
            },
            NameStyle::Plain => self.next_name(),
        };
        self.names.insert(id, name.clone());
        name
    }

    /// A readable identifier from `source`, suffixed (`greet2`, `greet3`, ...) until
    /// it collides with neither a reserved name nor a previously assigned one.
    fn unique_readable(&mut self, source: &str) -> String {
        let base = sanitize_identifier(source);
        let mut candidate = base.clone();
        let mut suffix = 2;
        while self.taken.contains(&candidate) {
            candidate = format!("{base}{suffix}");
            suffix += 1;
        }
        self.taken.insert(candidate.clone());
        candidate
    }

    fn next_idx(&mut self) -> u64 {
        let c = self.counter;
        self.counter += 1;
        c
    }

    fn next_name(&mut self) -> String {
        let c = self.next_idx();
        let short = self.name_from_idx(c);
        // In readable mode, temps are `$`-prefixed so they can't collide with a
        // readable (source-derived) name, which never contains `$`.
        match self.style {
            NameStyle::Readable => format!("${short}"),
            _ => short,
        }
    }

    fn name_from_idx(&self, n: u64) -> String {
        let mut s = String::new();
        let mut num = n;
        let base = self.chars.len() as u64;

        loop {
            let remainder = (num % base) as usize;
            s.push(self.chars[remainder]);
            num /= base;
            if num < 1 {
                break;
            }
            num -= 1;
        }

        s.chars().rev().collect()
    }
}

// --- Scope-aware name allocation --------------------------------------------
//
// The transform assigns each binding a globally-unique name. That's correct but
// not optimal: two locals named `value` in sibling functions become `value` and
// `value2`, and obfuscated names never reuse a letter across functions. This
// post-pass re-allocates names over the *JavaScript* scope tree so disjoint
// scopes share names: in readable mode both `value`s stay `value`; in release a
// short name is reused in every function.
//
// It runs on the assembled node tree, where the real lexical scopes are visible,
// so it's decoupled from any Vilan/JS scope mismatch. Scopes are function-grained
// (a block's `let`s belong to the enclosing function — safe, just less reuse).
// The collect walk may be incomplete (a missed binding just keeps its unique
// name); the rename walk must be exhaustive, so every node variant is handled.

/// The bindings declared directly in one JS function scope, plus its child
/// scopes (nested functions/closures). Names are the binding's current (unique)
/// output name, the key for the rename.
struct JsScope {
    declarations: Vec<String>,
    children: Vec<JsScope>,
}

/// `idx`th obfuscated short name (`a`, `b`, …, `aa`, …) — the release sequence.
fn short_name_from_idx(idx: u64) -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let base = CHARS.len() as u64;
    let mut bytes = Vec::new();
    let mut num = idx;
    loop {
        bytes.push(CHARS[(num % base) as usize]);
        num /= base;
        if num < 1 {
            break;
        }
        num -= 1;
    }
    bytes.reverse();
    String::from_utf8(bytes).unwrap()
}

/// The shortest obfuscated name not already in `used` (release allocation).
fn shortest_available(used: &HashSet<String>) -> String {
    let mut idx = 0;
    loop {
        let name = short_name_from_idx(idx);
        if !used.contains(&name) {
            return name;
        }
        idx += 1;
    }
}

/// `base`, or `base2`/`base3`/… if taken (readable allocation).
fn disambiguated(base: &str, used: &HashSet<String>) -> String {
    let base = sanitize_identifier(base);
    if !used.contains(&base) {
        return base;
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}{suffix}");
        if !used.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// The scope rooted at a function/closure: its parameters, then everything its
/// body declares.
fn function_scope(
    parameters: &[js::Parameter],
    body: &[js::Node],
    renameable: &HashSet<String>,
) -> JsScope {
    let mut declarations: Vec<String> = parameters
        .iter()
        .filter(|parameter| renameable.contains(&parameter.name))
        .map(|parameter| parameter.name.clone())
        .collect();
    let mut children = Vec::new();
    collect_declarations(body, renameable, &mut declarations, &mut children);
    JsScope {
        declarations,
        children,
    }
}

/// Collects, from a run of statements at one function level, the bindings
/// declared directly here (into `declarations`) and the nested function/closure
/// scopes (into `children`). Block bodies (`if`/`while`/`for`) are part of this
/// scope; functions and closures start child scopes.
fn collect_declarations(
    nodes: &[js::Node],
    renameable: &HashSet<String>,
    declarations: &mut Vec<String>,
    children: &mut Vec<JsScope>,
) {
    for node in nodes {
        collect_node(node, renameable, declarations, children);
    }
}

fn collect_node(
    node: &js::Node,
    renameable: &HashSet<String>,
    declarations: &mut Vec<String>,
    children: &mut Vec<JsScope>,
) {
    match node {
        js::Node::Function(function) => {
            if renameable.contains(&function.name) {
                declarations.push(function.name.clone());
            }
            children.push(function_scope(
                &function.parameters,
                &function.body,
                renameable,
            ));
        }
        js::Node::Closure(closure) => {
            children.push(function_scope(
                &closure.parameters,
                &closure.body,
                renameable,
            ));
        }
        js::Node::ConstVariable(variable) | js::Node::LetVariable(variable) => {
            if renameable.contains(&variable.name) {
                declarations.push(variable.name.clone());
            }
            collect_node(&variable.value, renameable, declarations, children);
        }
        js::Node::ForOf(binding, iterable, body) => {
            if renameable.contains(binding) {
                declarations.push(binding.clone());
            }
            collect_node(iterable, renameable, declarations, children);
            collect_declarations(body, renameable, declarations, children);
        }
        js::Node::While(condition, body) => {
            collect_node(condition, renameable, declarations, children);
            collect_declarations(body, renameable, declarations, children);
        }
        js::Node::If(branch) => collect_if(branch, renameable, declarations, children),
        js::Node::Call(subject, arguments) => {
            collect_node(subject, renameable, declarations, children);
            collect_declarations(arguments, renameable, declarations, children);
        }
        js::Node::Assignment(left, right)
        | js::Node::Binary(_, left, right)
        | js::Node::PropertyIndex(left, right) => {
            collect_node(left, renameable, declarations, children);
            collect_node(right, renameable, declarations, children);
        }
        js::Node::Await(inner)
        | js::Node::Unary(_, inner)
        | js::Node::Return(inner)
        | js::Node::Throw(inner)
        | js::Node::Spread(inner)
        | js::Node::Property(inner, _) => collect_node(inner, renameable, declarations, children),
        js::Node::Array(items) => collect_declarations(items, renameable, declarations, children),
        js::Node::Local(_)
        | js::Node::String(_)
        | js::Node::Number(_, _)
        | js::Node::Bool(_)
        | js::Node::Null
        | js::Node::Void
        | js::Node::Break
        | js::Node::Continue => {}
    }
}

fn collect_if(
    branch: &js::IfBranch,
    renameable: &HashSet<String>,
    declarations: &mut Vec<String>,
    children: &mut Vec<JsScope>,
) {
    match branch {
        js::IfBranch::If(condition, body, else_branch) => {
            collect_node(condition, renameable, declarations, children);
            collect_declarations(body, renameable, declarations, children);
            if let Some(else_branch) = else_branch {
                collect_if(else_branch, renameable, declarations, children);
            }
        }
        js::IfBranch::Else(body) => collect_declarations(body, renameable, declarations, children),
    }
}

/// Allocates names over the scope tree, top-down. A scope's bindings get names
/// not used by an ancestor (no shadowing) or a same-scope sibling; disjoint
/// scopes (passed the same inherited set) reuse freely. `release` picks the
/// shortest obfuscated name; otherwise the binding's source name, disambiguated.
fn allocate_scope(
    scope: &JsScope,
    inherited: &HashSet<String>,
    release: bool,
    source_of: &HashMap<String, String>,
    rename: &mut HashMap<String, String>,
) {
    let mut used = inherited.clone();
    for old in &scope.declarations {
        let new = if release {
            shortest_available(&used)
        } else {
            // Readable: `renameable` only holds source-named bindings, so this is
            // always present.
            disambiguated(source_of.get(old).unwrap_or(old), &used)
        };
        rename.insert(old.clone(), new.clone());
        used.insert(new);
    }
    for child in &scope.children {
        allocate_scope(child, &used, release, source_of, rename);
    }
}

/// Applies the rename map to every binding and reference in the tree. Property
/// names and untouched identifiers (externs, helpers — never in the map) are left
/// as-is. Must be exhaustive: a missed reference would dangle.
fn rename_nodes(nodes: &mut [js::Node], rename: &HashMap<String, String>) {
    for node in nodes {
        rename_node(node, rename);
    }
}

fn rename_one(name: &mut String, rename: &HashMap<String, String>) {
    if let Some(new) = rename.get(name) {
        *name = new.clone();
    }
}

fn rename_node(node: &mut js::Node, rename: &HashMap<String, String>) {
    match node {
        js::Node::Local(name) => rename_one(name, rename),
        js::Node::Function(function) => {
            rename_one(&mut function.name, rename);
            for parameter in &mut function.parameters {
                rename_one(&mut parameter.name, rename);
            }
            rename_nodes(&mut function.body, rename);
        }
        js::Node::Closure(closure) => {
            for parameter in &mut closure.parameters {
                rename_one(&mut parameter.name, rename);
            }
            rename_nodes(&mut closure.body, rename);
        }
        js::Node::ConstVariable(variable) | js::Node::LetVariable(variable) => {
            rename_one(&mut variable.name, rename);
            rename_node(&mut variable.value, rename);
        }
        js::Node::ForOf(binding, iterable, body) => {
            rename_one(binding, rename);
            rename_node(iterable, rename);
            rename_nodes(body, rename);
        }
        js::Node::While(condition, body) => {
            rename_node(condition, rename);
            rename_nodes(body, rename);
        }
        js::Node::If(branch) => rename_if(branch, rename),
        js::Node::Call(subject, arguments) => {
            rename_node(subject, rename);
            rename_nodes(arguments, rename);
        }
        js::Node::Assignment(left, right)
        | js::Node::Binary(_, left, right)
        | js::Node::PropertyIndex(left, right) => {
            rename_node(left, rename);
            rename_node(right, rename);
        }
        js::Node::Await(inner)
        | js::Node::Unary(_, inner)
        | js::Node::Return(inner)
        | js::Node::Throw(inner)
        | js::Node::Spread(inner)
        // `Property`'s member is a property name, not a binding — recurse only the subject.
        | js::Node::Property(inner, _) => rename_node(inner, rename),
        js::Node::Array(items) => rename_nodes(items, rename),
        js::Node::String(_)
        | js::Node::Number(_, _)
        | js::Node::Bool(_)
        | js::Node::Null
        | js::Node::Void
        | js::Node::Break
        | js::Node::Continue => {}
    }
}

fn rename_if(branch: &mut js::IfBranch, rename: &HashMap<String, String>) {
    match branch {
        js::IfBranch::If(condition, body, else_branch) => {
            rename_node(condition, rename);
            rename_nodes(body, rename);
            if let Some(else_branch) = else_branch {
                rename_if(else_branch, rename);
            }
        }
        js::IfBranch::Else(body) => rename_nodes(body, rename),
    }
}

/// Re-allocates the program's binding names over its JS scope tree (see the
/// machinery above) and rewrites the node tree. A no-op for the annotated style
/// (its names carry `/*source*/` comments the rename can't cleanly reuse).
fn rename_for_scopes(ng: &NameGenerator, program: &Program, nodes: &mut Vec<js::Node>) {
    let release = match ng.style {
        NameStyle::Annotated => return,
        NameStyle::Readable => false,
        NameStyle::Plain => true,
    };
    // Each renameable binding's current (unique) name -> its source name.
    let mut source_of: HashMap<String, String> = HashMap::new();
    for (id, name) in &ng.names {
        if let Some(source) = ng.source_names.get(id) {
            source_of.insert(name.clone(), source.clone());
        }
    }
    // Readable reuses only source-named bindings (anonymous temps keep their
    // unique `$`-name); release reuses every generated name.
    let renameable: HashSet<String> = if release {
        ng.names.values().cloned().collect()
    } else {
        source_of.keys().cloned().collect()
    };
    if renameable.is_empty() {
        return;
    }
    // The reserved set (keywords, referenced globals, `__`-helpers, the program's
    // `[extern]` symbols) counts as used in every scope, so nothing collides.
    let reserved = collect_reserved_names(program);
    let mut declarations = Vec::new();
    let mut children = Vec::new();
    collect_declarations(nodes, &renameable, &mut declarations, &mut children);
    let global = JsScope {
        declarations,
        children,
    };
    let mut rename = HashMap::new();
    allocate_scope(&global, &reserved, release, &source_of, &mut rename);
    rename_nodes(nodes, &rename);
}
