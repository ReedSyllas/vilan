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
