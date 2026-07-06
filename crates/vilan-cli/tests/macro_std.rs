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
    let item = Item::Struct(StructItem { name = "Point", fields = [Field { name = "x", type_ = TypeExpr { name = "i32", arguments = [] } }] });
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
