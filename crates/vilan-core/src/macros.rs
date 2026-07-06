//! The macro engine's expansion layer (proposal/macro-engine.md, Phase 1):
//! `macro fun` definitions compile in per-file HERMETIC worlds and run in the
//! expansion interpreter; `[name(args)]` attributes (and `[derive(Name)]` for
//! registered names) splice each macro's returned `Source` text into the
//! program before analysis.
//!
//! The macro world (§3): a file's `macro fun`s are compiled from a BLANKED
//! copy of that file — every byte outside the macro definitions becomes a
//! space (newlines kept), and the `macro` keyword itself is blanked, leaving
//! plain `fun`s at their original offsets. Spans in world diagnostics therefore
//! point at the true positions in the user's file. The world's package
//! universe is `macro_std` alone (a workspace with that single dependency);
//! hermeticity of the bodies is checked syntactically here (imports must root
//! at `macro_std`) and physically by construction (nothing else of the user's
//! program exists in the world).
//!
//! Caching (§6): worlds are cached by blanked-file content hash; expansions by
//! (world, macro, item source, argument sources) — sound because the
//! interpreter is deterministic by construction. Both caches hold leaked,
//! process-global data, mirroring `load_package_module`'s parse cache.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::Error;
use crate::interpreter::{self, Limits};
use crate::node::{Func, ImportBranch, Node, NodeList, Pattern};
use crate::options::BuildOptions;
use crate::span::{Span, Spanned};
use crate::transformer::{JsProgram, js, transform_functions};
use crate::{PackageSpec, Platform, Workspace, analyze_source};

/// Expansion depth cap: a macro's output may carry further attribute
/// invocations; past this the chain is reported instead of chased.
const MAX_DEPTH: u32 = 16;

/// A compiled per-file macro world: the transformed program the interpreter
/// executes, plus each macro's emitted entry name and parameter count.
pub(crate) struct World {
    key: u64,
    program: JsProgram<'static>,
    entries: HashMap<String, (String, usize)>,
}

#[derive(Default)]
pub(crate) struct MacroRegistry {
    by_name: HashMap<String, MacroDef>,
}

pub(crate) struct MacroDef {
    world: Arc<World>,
    entry: String,
    parameters: usize,
}

/// The `macro_std` package, resolved from its toolchain location beside `std`
/// (`<roots>/std/src` → `<roots>/macro_std`). `None` when absent — an error is
/// reported only when a program actually defines a macro.
pub(crate) fn resolve_macro_std(std: &PackageSpec) -> Option<PackageSpec> {
    let dir = std.base_root.parent()?.parent()?.join("macro_std");
    dir.join("vilan.toml")
        .is_file()
        .then(|| crate::manifest::resolve_std(&dir))
}

/// The top-level `macro fun`s of a file, with each definition's full span.
fn macro_funs<'a, 'src>(nodes: &'a NodeList<'src>) -> Vec<(&'a Func<'src>, Span)> {
    nodes
        .iter()
        .filter_map(|(node, span)| match node {
            Node::MacroFun(function) => Some((function, *span)),
            Node::Export(inner) => match &inner.0 {
                Node::MacroFun(function) => Some((function, inner.1)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Registers a file's `macro fun`s: checks their bodies' hermeticity, compiles
/// the file's macro world (cached by blanked content), and binds each macro
/// name. Diagnostics carry spans into THIS file (the caller attributes them).
pub(crate) fn register_file(
    registry: &mut MacroRegistry,
    nodes: &NodeList,
    text: &str,
    file: &Path,
    std: &PackageSpec,
    diagnostics: &mut Vec<Error>,
) {
    let definitions = macro_funs(nodes);
    if definitions.is_empty() {
        return;
    }
    let Some(macro_std) = resolve_macro_std(std) else {
        diagnostics.push(Error {
            span: definitions[0].1,
            msg: "the `macro_std` package was not found beside `std` — macros need the \
                  toolchain's `macro_std`"
                .to_string(),
        });
        return;
    };

    // Hermeticity (§4): a macro body imports only from `macro_std`. Checked on
    // the ORIGINAL parse so the error points at the offending import.
    let mut hermetic = true;
    for (function, _) in &definitions {
        if let Some(body) = &function.body {
            for statement in body.0.0.iter().chain(std::iter::once(&*body.0.1)) {
                check_hermetic_imports(statement, diagnostics, &mut hermetic);
            }
        }
    }
    if !hermetic {
        return;
    }

    let blanked = blank_to_world(text, &definitions);
    let world = match compile_world(blanked, file, std, &macro_std, &definitions) {
        Ok(world) => world,
        Err(errors) => {
            diagnostics.extend(errors);
            return;
        }
    };
    for (function, _) in &definitions {
        let name = function.name.0.to_string();
        if registry.by_name.contains_key(&name) {
            diagnostics.push(Error {
                span: function.name.1,
                msg: format!("a macro named `{name}` is already defined"),
            });
            continue;
        }
        let Some((entry, parameters)) = world.entries.get(&name).cloned() else {
            continue; // the world compile reported why
        };
        registry.by_name.insert(
            name,
            MacroDef {
                world: world.clone(),
                entry,
                parameters,
            },
        );
    }
}

fn check_hermetic_imports(node: &Spanned<Node>, diagnostics: &mut Vec<Error>, hermetic: &mut bool) {
    if let Node::Import(branch) | Node::Use(branch) = &node.0 {
        let root = match branch {
            ImportBranch::Path(root, _, _) => Some(*root),
            ImportBranch::Set(_) => None,
        };
        if root != Some("macro_std") {
            diagnostics.push(Error {
                span: node.1,
                msg: "a macro body may import only from `macro_std` — the macro world is \
                      hermetic (macro-engine.md §4)"
                    .to_string(),
            });
            *hermetic = false;
        }
    }
    node.0
        .for_each_child(&mut |child| check_hermetic_imports(child, diagnostics, hermetic));
}

/// The macro world's source: every byte outside the macro definitions becomes
/// a space (newlines kept, so spans and line numbers stay true), and each
/// definition's leading `macro` keyword is blanked, leaving a plain `fun`.
fn blank_to_world(text: &str, definitions: &[(&Func, Span)]) -> String {
    let bytes = text.as_bytes();
    let mut blanked: Vec<u8> = bytes
        .iter()
        .map(|&b| if b == b'\n' { b'\n' } else { b' ' })
        .collect();
    for (_, span) in definitions {
        let range = span.into_range();
        let range = range.start.min(bytes.len())..range.end.min(bytes.len());
        blanked[range.clone()].copy_from_slice(&bytes[range.clone()]);
        // `macro` → 5 spaces (the definition span starts at the keyword).
        if bytes[range.clone()].starts_with(b"macro") {
            blanked[range.start..range.start + 5].fill(b' ');
        }
    }
    String::from_utf8(blanked).expect("blanking preserves UTF-8 (multibyte bytes become spaces)")
}

fn content_key(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn compile_world(
    blanked: String,
    file: &Path,
    std: &PackageSpec,
    macro_std: &PackageSpec,
    definitions: &[(&Func, Span)],
) -> Result<Arc<World>, Vec<Error>> {
    static WORLDS: OnceLock<Mutex<HashMap<u64, Arc<World>>>> = OnceLock::new();
    let key = content_key(&blanked);
    let worlds = WORLDS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(world) = worlds.lock().unwrap().get(&key) {
        return Ok(world.clone());
    }

    let leaked: &'static str = Box::leak(blanked.into_boxed_str());
    let workspace = Workspace {
        packages: vec![macro_std.clone()],
        entry_dependencies: vec![("macro_std".to_string(), 0)],
    };
    let (program, errors) = analyze_source(
        leaked,
        std,
        file.parent().unwrap_or(Path::new(".")),
        file,
        Some(Platform::default()),
        &workspace,
    );
    if !errors.is_empty() {
        return Err(errors
            .into_iter()
            .map(|error| Error {
                span: error.span,
                msg: format!("in this macro: {}", error.msg),
            })
            .collect());
    }
    let Some(program) = program else {
        return Err(vec![Error {
            span: definitions[0].1,
            msg: "the macro world failed to compile".to_string(),
        }]);
    };
    // The world outlives this analysis (it's cached process-globally), so the
    // program is leaked — bounded: one leak per distinct macro-definition set,
    // the same shape as the parse cache's leaks.
    let program: &'static crate::Program<'static> = Box::leak(Box::new(program));

    let mut roots = Vec::new();
    let mut root_names = Vec::new();
    let mut errors = Vec::new();
    for (function, span) in definitions {
        let name = function.name.0;
        let id = program
            .scopes
            .get(&program.global_scope_id)
            .and_then(|scope| scope.name_to_id_map.get(name))
            .copied();
        match id {
            Some(id) if program.functions.contains_key(&id) => {
                roots.push(id);
                root_names.push((name.to_string(), function.parameters.0.len()));
            }
            _ => errors.push(Error {
                span: *span,
                msg: format!("the macro `{name}` did not compile to a callable function"),
            }),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let (js_program, emitted) = transform_functions(program, &BuildOptions::default(), &roots)
        .map_err(|error| vec![error])?;
    let entries = roots
        .iter()
        .zip(root_names)
        .map(|(id, (name, parameters))| {
            (
                name,
                (emitted.get(id).cloned().unwrap_or_default(), parameters),
            )
        })
        .collect();
    let world = Arc::new(World {
        key,
        program: js_program,
        entries,
    });
    worlds.lock().unwrap().insert(key, world.clone());
    Ok(world)
}

// --- Expansion ---

/// Expands every macro attribute in `nodes` (a file's top level, or a
/// generated list when recursing): each hit runs its macro in the interpreter
/// and parses the returned `Source` text. Returns the generated node lists to
/// be walked after the originating file's items — including nested expansions
/// and the built-in derives the generated code carries.
pub(crate) fn expand_source(
    registry: &MacroRegistry,
    nodes: &NodeList,
    text: &str,
    diagnostics: &mut Vec<Error>,
    depth: u32,
) -> Vec<&'static NodeList<'static>> {
    let mut generated = Vec::new();
    for node in nodes {
        expand_node(registry, node, text, diagnostics, depth, &mut generated);
    }
    generated
}

fn expand_node(
    registry: &MacroRegistry,
    node: &Spanned<Node>,
    text: &str,
    diagnostics: &mut Vec<Error>,
    depth: u32,
    generated: &mut Vec<&'static NodeList<'static>>,
) {
    match &node.0 {
        Node::Export(inner) => expand_node(registry, inner, text, diagnostics, depth, generated),
        // `mod` bodies are item position too.
        Node::Module(_, body) => {
            for child in &body.0 {
                expand_node(registry, child, text, diagnostics, depth, generated);
            }
        }
        Node::MacroAttribute(name, name_span, argument_spans, item) => {
            let Some(def) = registry.by_name.get(*name) else {
                diagnostics.push(Error {
                    span: *name_span,
                    msg: format!("no macro named `{name}` is in scope"),
                });
                return;
            };
            let arguments: Vec<&str> = argument_spans
                .iter()
                .map(|span| slice(text, *span))
                .collect();
            run_expansion(
                registry,
                def,
                name,
                *name_span,
                item,
                &arguments,
                text,
                diagnostics,
                depth,
                generated,
            );
        }
        // `[derive(Name)]` with a registered name dispatches like an attribute
        // with no arguments; built-in names keep their Rust generators, and
        // unknown names keep today's behavior (skip — the missing-impl error
        // surfaces at the use site).
        Node::Derive(names, item) => {
            for name in names {
                if let Some(def) = registry.by_name.get(*name) {
                    run_expansion(
                        registry,
                        def,
                        name,
                        node.1,
                        item,
                        &[],
                        text,
                        diagnostics,
                        depth,
                        generated,
                    );
                }
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn run_expansion(
    registry: &MacroRegistry,
    def: &MacroDef,
    name: &str,
    site: Span,
    item: &Spanned<Node>,
    arguments: &[&str],
    text: &str,
    diagnostics: &mut Vec<Error>,
    depth: u32,
    generated: &mut Vec<&'static NodeList<'static>>,
) {
    if depth >= MAX_DEPTH {
        diagnostics.push(Error {
            span: site,
            msg: format!(
                "macro expansion did not settle after {MAX_DEPTH} rounds — the chain ends at \
                 `{name}`"
            ),
        });
        return;
    }

    // The cache key: the world (definition set), the macro, and the invocation
    // input AS SOURCE — the item's text and the argument texts (§6). The cached
    // value is the parsed output with its (leaked) text, so re-analyses skip
    // the interpreter AND the parse.
    static EXPANSIONS: OnceLock<Mutex<HashMap<u64, (&'static NodeList<'static>, &'static str)>>> =
        OnceLock::new();
    let item_text = slice(text, item.1);
    let key = {
        let mut hasher = DefaultHasher::new();
        def.world.key.hash(&mut hasher);
        name.hash(&mut hasher);
        item_text.hash(&mut hasher);
        arguments.hash(&mut hasher);
        hasher.finish()
    };
    let expansions = EXPANSIONS.get_or_init(|| Mutex::new(HashMap::new()));
    let cached = expansions.lock().unwrap().get(&key).copied();
    let (parsed, parsed_source) = match cached {
        Some(cached) => cached,
        None => {
            let mut call_arguments = vec![construct_item(item, text)];
            if def.parameters >= 2 {
                call_arguments.push(construct_arguments(arguments));
            }
            let source = match interpreter::run_entry(
                &def.world.program,
                &def.entry,
                &call_arguments,
                Limits::default(),
            ) {
                Ok(source) => source,
                Err(failure) => {
                    diagnostics.push(Error {
                        span: site,
                        msg: format!(
                            "macro `{name}` failed at expansion time: {}",
                            failure.message
                        ),
                    });
                    return;
                }
            };
            let (parsed, parsed_source) = match parse_generated(&source) {
                Ok(parsed) => parsed,
                Err(message) => {
                    diagnostics.push(Error {
                        span: site,
                        msg: format!("macro `{name}` generated invalid vilan ({message})"),
                    });
                    return;
                }
            };
            if !macro_funs(parsed).is_empty() {
                diagnostics.push(Error {
                    span: site,
                    msg: format!(
                        "macro `{name}` generated a `macro fun` — macros cannot define macros \
                         (macro-engine.md §3)"
                    ),
                });
                return;
            }
            expansions
                .lock()
                .unwrap()
                .insert(key, (parsed, parsed_source));
            (parsed, parsed_source)
        }
    };
    generated.push(parsed);
    // The generated code may carry built-in derives and further macro
    // attributes: expand both, one level deeper.
    if let Some(derived) = crate::analyzer::expand_derives(parsed) {
        generated.push(derived);
    }
    let nested = expand_source(registry, parsed, parsed_source, diagnostics, depth + 1);
    generated.extend(nested);
}

fn slice<'a>(text: &'a str, span: Span) -> &'a str {
    let range = span.into_range();
    text.get(range.start.min(text.len())..range.end.min(text.len()))
        .unwrap_or("")
}

/// Lexes + parses a macro's returned source. Unlike the trusted derive
/// generators, macro output is user code: errors are returned, not swallowed.
fn parse_generated(source: &str) -> Result<(&'static NodeList<'static>, &'static str), String> {
    use chumsky::prelude::*;
    let source: &'static str = Box::leak(source.to_string().into_boxed_str());
    let (tokens, lex_errors) = crate::lexer::lexer().parse(source).into_output_errors();
    if let Some(error) = lex_errors.first() {
        return Err(error.to_string());
    }
    let Some(tokens) = tokens else {
        return Err("empty output".to_string());
    };
    let end = source.len();
    let (root, parse_errors) = crate::parser::parser()
        .map_with(|ast, e| (ast, e.span()))
        .parse(
            tokens
                .as_slice()
                .map((end..end).into(), |(token, span)| (token, span)),
        )
        .into_output_errors();
    if let Some(error) = parse_errors.first() {
        return Err(error.to_string());
    }
    match root {
        Some((root, _file_span)) => {
            let leaked: &'static crate::span::Spanned<NodeList<'static>> =
                &*Box::leak(Box::new(root));
            Ok((&leaked.0, source))
        }
        None => Err("empty output".to_string()),
    }
}

// --- Reflection literals (the meta.vl layout contract) ---
//
// `macro_std::meta`'s types are ordinary vilan values, so at the JS level a
// struct is its fields as a positional array (declaration order) and an enum
// variant is `[discriminant, ...payload]`. The constructors below build the
// interpreter's inputs in exactly that layout; `meta.vl` and this section
// change together (the end-to-end corpus test pins the pair).

fn string_literal(text: &str) -> js::Node<'static> {
    js::Node::String(std::borrow::Cow::Owned(text.to_string()))
}

fn array(items: Vec<js::Node<'static>>) -> js::Node<'static> {
    js::Node::Array(items)
}

fn discriminant(index: usize) -> js::Node<'static> {
    js::Node::Number(index.to_string(), None)
}

/// `TypeExpr { name, arguments }` from a written type. Named heads keep their
/// arguments; every other type form (tuples, closures, references, mapped
/// types) becomes its source text as the name with no arguments — renderable
/// back verbatim, which is all v1's syntactic macros need.
fn construct_type_expr(node: &Spanned<Node>, text: &str) -> js::Node<'static> {
    match &node.0 {
        Node::Accessor(name) => array(vec![string_literal(name), array(Vec::new())]),
        Node::AccessorWithGenerics(name, arguments) => array(vec![
            string_literal(name),
            array(
                arguments
                    .0
                    .iter()
                    .map(|argument| construct_type_expr(argument, text))
                    .collect(),
            ),
        ]),
        _ => array(vec![string_literal(slice(text, node.1)), array(Vec::new())]),
    }
}

fn void_type_expr() -> js::Node<'static> {
    array(vec![string_literal("void"), array(Vec::new())])
}

/// The `Item` value for the annotated node: `[0, StructItem]`, `[1, EnumItem]`,
/// or `[2, FunctionItem]` — the variant order declared in `meta.vl`.
fn construct_item(item: &Spanned<Node>, text: &str) -> js::Node<'static> {
    match &item.0 {
        Node::Struct(name, _generics, _external, fields) => {
            let fields = fields
                .iter()
                .flat_map(|fields| &fields.0)
                .map(|(field, _)| {
                    let (field_name, field_type, _exposed) = field;
                    array(vec![
                        string_literal(field_name.0),
                        field_type
                            .as_ref()
                            .map(|type_| construct_type_expr(type_, text))
                            .unwrap_or_else(void_type_expr),
                    ])
                })
                .collect();
            array(vec![
                discriminant(0),
                array(vec![string_literal(name.0), array(fields)]),
            ])
        }
        Node::Enum(name, _generics, variants) => {
            let variants = variants
                .0
                .iter()
                .map(|(variant, _)| {
                    let (variant_name, payload, _discriminant) = variant;
                    array(vec![
                        string_literal(variant_name),
                        array(
                            payload
                                .iter()
                                .map(|type_| construct_type_expr(type_, text))
                                .collect(),
                        ),
                    ])
                })
                .collect();
            array(vec![
                discriminant(1),
                array(vec![string_literal(name.0), array(variants)]),
            ])
        }
        Node::Func(function) => {
            let parameters = function
                .parameters
                .0
                .iter()
                .map(|(pattern, type_, _convention, span)| {
                    let parameter_name = match pattern {
                        Pattern::Binding(name, _) => (*name).to_string(),
                        _ => slice(text, *span).to_string(),
                    };
                    array(vec![
                        string_literal(&parameter_name),
                        type_
                            .as_ref()
                            .map(|type_| construct_type_expr(type_, text))
                            .unwrap_or_else(void_type_expr),
                    ])
                })
                .collect();
            array(vec![
                discriminant(2),
                array(vec![
                    string_literal(function.name.0),
                    array(parameters),
                    function
                        .return_type
                        .as_ref()
                        .map(|type_| construct_type_expr(type_, text))
                        .unwrap_or_else(void_type_expr),
                ]),
            ])
        }
        // The parser only puts structs/enums/functions under an attribute.
        _ => array(vec![
            discriminant(0),
            array(vec![string_literal(""), array(Vec::new())]),
        ]),
    }
}

/// `Arguments { values }` — the invocation's argument source texts.
fn construct_arguments(arguments: &[&str]) -> js::Node<'static> {
    array(vec![array(
        arguments
            .iter()
            .map(|argument| string_literal(argument.trim()))
            .collect(),
    )])
}
