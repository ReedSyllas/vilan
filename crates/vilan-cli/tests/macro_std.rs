//! `macro_std` (proposal/macro-engine.md §3, Phase 0): the macro world's std.
//! Until the hermetic resolver lands (Phase 1), the package is exercised as an
//! ordinary path dependency — this pins that its reflection surface compiles,
//! constructs, and dispatches end to end.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vilan_macro_std_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn write(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn vilan(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(args)
        .output()
        .expect("run vilan")
}

#[test]
fn the_reflection_surface_works_end_to_end() {
    let macro_std = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/macro_std");
    let macro_std = macro_std.canonicalize().expect("macro_std path");
    let dir = temp_project("surface");
    write(&dir, "vilan.toml", "[project]\npackages = [\"app\"]\n");
    write(
        &dir,
        "app/vilan.toml",
        &format!(
            "[package]\nname = \"app\"\ntarget = \"node\"\n\n[package.dependencies]\nmacro_std = {{ path = \"{}\" }}\n",
            macro_std.display()
        ),
    );
    write(
        &dir,
        "app/src/main.vl",
        r#"import std::print;
import std::option::Option::{ self, Some, None };
import macro_std::source;
import macro_std::meta::{ TypeExpr, Item, StructItem, Field };

fun main() {
    let list_of_i32 = TypeExpr { name = "List", arguments = [TypeExpr { name = "i32", arguments = [] }] };
    print(list_of_i32.render());
    print(source("let x = 1;").text);
    let item = Item::Struct(StructItem { name = "Point", fields = [Field { name = "x", type_ = TypeExpr { name = "i32", arguments = [] }, exposed = false }] });
    match item.as_struct() {
        Some(let found) => print(found.name),
        None => print("not a struct"),
    }
    match item.as_enum() {
        Some(let found) => print(found.name),
        None => print("not an enum"),
    }
}

main();
"#,
    );
    let output = vilan(&["build", dir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new("node")
        .arg(dir.join("dist/app.js"))
        .output()
        .expect("run node");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "List<i32>\nlet x = 1;\nPoint\nnot an enum\n",
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// Phase 1's exit criterion (macro-engine.md §11): a macro DEFINED IN A LIBRARY
// drives generation in the app that depends on it.
#[test]
fn a_library_macro_expands_in_the_consuming_app() {
    let macro_std = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vilan/macro_std");
    let _ = macro_std.canonicalize().expect("macro_std path");
    let dir = temp_project("library_macro");
    write(
        &dir,
        "vilan.toml",
        "[project]\npackages = [\"macros\", \"app\"]\n",
    );
    write(&dir, "macros/vilan.toml", "[library]\nname = \"macros\"\n");
    write(
        &dir,
        "macros/src/lib.vl",
        r#"macro fun derive_tag(item: Item): Source {
	import macro_std::source;
	import macro_std::meta::{ Item, Source, StructItem };
	import macro_std::option::Option::{ self, Some, None };

	let target = match item.as_struct() {
		Some(let found) => found,
		None => StructItem { name = "?", fields = [] },
	};
	source("impl " + target.name + " {\nfun tag(self): str {\n\"" + target.name + "\"\n}\n}\n")
}
"#,
    );
    write(
        &dir,
        "app/vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n\n[package.dependencies]\nmacros = { path = \"../macros\" }\n",
    );
    // A bare MODULE import does not bring the macro into scope (names are
    // module-scoped now): the build fails, naming the missing macro.
    write(
        &dir,
        "app/src/main.vl",
        r#"import std::print;
import macros;

[derive_tag]
struct Widget {
	size: i32,
}

fun main() {
	print(Widget { size = 1 }.tag());
}

main();
"#,
    );
    let output = vilan(&["build", dir.to_str().unwrap()]);
    assert!(!output.status.success(), "a module import must not suffice");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        text.contains("no macro named `derive_tag` is in scope"),
        "unexpected output: {text}"
    );
    // The LEAF import brings it into scope.
    write(
        &dir,
        "app/src/main.vl",
        r#"import std::print;
import macros::derive_tag;

[derive_tag]
struct Widget {
	size: i32,
}

fun main() {
	print(Widget { size = 1 }.tag());
}

main();
"#,
    );
    let output = vilan(&["build", dir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new("node")
        .arg(dir.join("dist/app.js"))
        .output()
        .expect("run node");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "Widget\n",
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// `vilan.toml [macro]` (macro-engine.md §5/§12): the per-package fuel budget.
// A starved budget stops an expensive macro with the clean expansion-time
// error; a raised one lets it finish.
#[test]
fn the_macro_section_configures_fuel() {
    let source = r#"macro fun expensive(item: Item): Source {
	import macro_std::source;
	import macro_std::meta::{ Item, Source };

	mut n = 0;
	for n < 100000 {
		n = n + 1;
	}
	source("")
}

[expensive]
struct Point {
	x: i32,
}

fun main() {}

main();
"#;
    for (fuel, expect_success) in [("2000", false), ("5000000", true)] {
        let dir = temp_project(&format!("fuel_{fuel}"));
        write(
            &dir,
            "vilan.toml",
            &format!("[package]\nname = \"app\"\ntarget = \"node\"\n\n[macro]\nfuel = {fuel}\n"),
        );
        write(&dir, "src/main.vl", source);
        let output = vilan(&["build", dir.to_str().unwrap()]);
        if expect_success {
            assert!(
                output.status.success(),
                "fuel {fuel} should suffice: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        } else {
            assert!(!output.status.success(), "fuel {fuel} should exhaust");
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                text.contains("failed at expansion time") && text.contains("fuel"),
                "unexpected output: {text}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// `macro_std::build` (macro-engine.md §3, construction API step 2): the output
// builders render the exact shapes the derives are written against, and a
// builder-built impl splices and runs. The report functions return the rendered
// text verbatim, so this pins the BYTES of every shape — impl/fun nesting,
// match arms and arm blocks, quoting, struct literals and declarations — not
// just their behavior.
#[test]
fn the_output_builders_render_and_splice() {
    let dir = temp_project("builders");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"app\"\ntarget = \"node\"\n",
    );
    write(
        &dir,
        "src/main.vl",
        r#"import std::print;

macro fun make_shape(): Source {
	import macro_std::source;
	import macro_std::meta::Source;
	import macro_std::build::{ impl_of, fun_of, match_of, init_of, struct_of, quote, join };

	mut comparisons: List<str> = [];
	comparisons.push("self.x == other.x");
	comparisons.push("self.y == other.y");
	let eq = fun_of("eq")
		.parameter("self")
		.parameter("other: Probe")
		.returns("bool")
		.expr(join(comparisons, " && "));
	let shape = impl_of("Probe").implements("Marker").method(eq).render();

	let arms = match_of("(self, other)")
		.arm("(P::A, P::A)", "true")
		.arm_block("(P::B(let s0), P::B(let o0))", ["print(s0);", "s0 == o0"])
		.arm("_", "false")
		.render();
	let quoting = quote("say \"hi\"\\now");
	let literal = init_of("Probe").field("x", "1").field("y", "2").render();
	let declaration = struct_of("Wrapper").generics("<T: Marker>").field("inner: T").render();
	let report = fun_of("generated_report")
		.returns("str")
		.expr(quote(shape + "===" + arms + "===" + quoting + "===" + literal + "===" + declaration))
		.render();
	source(shape + report + "\n")
}

macro fun edge_report(): Source {
	import macro_std::source;
	import macro_std::meta::Source;
	import macro_std::build::{ impl_of, fun_of, match_of, quote, join, indent };

	mut empty: List<str> = [];
	let joined = join(empty, ", ");
	let indented = indent("a\n\nb", 2);
	let quoted = quote("");
	let bare_fun = fun_of("f").render();
	let bare_match = match_of("x").render();
	let bare_impl = impl_of("T").render();
	let edges = fun_of("edge_cases")
		.returns("str")
		.expr(quote(joined + "|" + indented + "|" + quoted + "|" + bare_fun + "|" + bare_match + "|" + bare_impl))
		.render();
	source(edges + "\n")
}

macro make_shape()
macro edge_report()

trait Marker {
	fun eq(self, other: Probe): bool;
}

struct Probe { x: i32, y: i32 }

fun main() {
	print(generated_report());
	print(edge_cases());
	let a = Probe { x = 1, y = 2 };
	let b = Probe { x = 1, y = 2 };
	print(a.eq(b));
}

main();
"#,
    );
    let output = vilan(&["build", dir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "build failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new("node")
        .arg(dir.join("src/main.js"))
        .output()
        .expect("run node");
    let shape = "impl Probe with Marker {\n\tfun eq(self, other: Probe): bool {\n\t\tself.x == other.x && self.y == other.y\n\t}\n}\n";
    let arms = "match (self, other) {\n\t(P::A, P::A) => true,\n\t(P::B(let s0), P::B(let o0)) => {\n\t\tprint(s0);\n\t\ts0 == o0\n\t},\n\t_ => false,\n}";
    let quoting = "\"say \\\"hi\\\"\\\\now\"";
    let literal = "Probe { x = 1, y = 2 }";
    let declaration = "struct Wrapper<T: Marker> {\n\tinner: T,\n}\n";
    let edges = "|\t\ta\n\n\t\tb|\"\"|fun f() {\n}|match x {\n}|impl T {\n}\n";
    let expected =
        format!("{shape}==={arms}==={quoting}==={literal}==={declaration}\n{edges}\ntrue\n");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected,
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
