//! End-to-end runtime test for `std::router` + `View.swap` (backlog A10,
//! proposal/router.md): a browser-target app is built with the real CLI and
//! run under node against a ~60-line DOM/history stub, asserting the routing
//! semantics the corpus can't reach (it only runs process-platform programs):
//! parse-on-load, typed `link` hrefs, plain-click interception (and modifier
//! passthrough), `pushState`/`popstate` driving one signal, nested layouts
//! through `swap`, the `PartialEq` no-op on an unchanged route, and disposal
//! of a swapped-out subtree's subscriptions.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh temp directory for the test's project tree.
fn temp_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vilan_router_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Writes `contents` to `dir/relative`, creating parent directories.
fn write(dir: &Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// The app under test: the Kolt-shaped route space — nested enums mirroring
/// nested layouts, a hand-written `parse`/`href` pair over `segments`, typed
/// `link`s, programmatic `navigate`, and a `swap`-rendered page tree.
const APP: &str = r#"import std::ui::{ View, view, mount_root };
import std::reactive::Signal;
import std::router::{ current_path, navigate, segments, link, Routable };
import std::option::Option::{ self, Some, None };

[derive(PartialEq)]
enum Route {
	Home,
	Login,
	Workspace(str, WorkspaceRoute),
	NotFound,
}

[derive(PartialEq)]
enum WorkspaceRoute {
	Overview,
	Tasks,
	Task(i32),
}

fun parse(path: str): Route {
	let parts = segments(path);
	match parts.len() {
		0 => Route::Home,
		1 => if parts[0] == "login" { Route::Login } else { Route::NotFound },
		_ => {
			if parts[0] == "w" {
				Route::Workspace(parts[1], parse_workspace(parts))
			} else {
				Route::NotFound
			}
		},
	}
}

fun parse_workspace(parts: List<str>): WorkspaceRoute {
	if parts.len() == 2 {
		WorkspaceRoute::Overview
	} else if parts[2] == "tasks" {
		WorkspaceRoute::Tasks
	} else if parts[2] == "task" && parts.len() > 3 {
		match parts[3].parse_i32() {
			Some(let id) => WorkspaceRoute::Task(id),
			None => WorkspaceRoute::Overview,
		}
	} else {
		WorkspaceRoute::Overview
	}
}

fun href(route: Route): str {
	match route {
		Route::Home => "/",
		Route::Login => "/login",
		Route::Workspace(let org, let inner) => i"/w/{org}" + workspace_href(inner),
		Route::NotFound => "/404",
	}
}

fun workspace_href(inner: WorkspaceRoute): str {
	match inner {
		WorkspaceRoute::Overview => "",
		WorkspaceRoute::Tasks => "/tasks",
		WorkspaceRoute::Task(let id) => i"/task/{id}",
	}
}

impl Route with Routable {
	fun to_path(self): str {
		href(self)
	}
}

fun home_page(): View {
	view("section").text("home")
}

fun workspace_layout(org: str, inner: WorkspaceRoute): View {
	view("section")
		.child(view("aside").text(org))
		.child(match inner {
			WorkspaceRoute::Overview => view("div").text("overview"),
			WorkspaceRoute::Tasks => view("div").text("tasks"),
			WorkspaceRoute::Task(let id) => view("div").text(i"task {id}"),
		})
}

fun app(route: Signal<Route>): View {
	view("main")
		.child(view("nav")
			.child(link("Home", Route::Home))
			.child(link("Tasks", Route::Workspace("acme", WorkspaceRoute::Tasks))))
		.child(view("button").text("go").on("click", || navigate(href(Route::Login))))
		.swap(route, |current| match current {
			Route::Home => home_page(),
			Route::Login => view("section").bind_text(current_path()),
			Route::Workspace(let org, let inner) => workspace_layout(org, inner),
			Route::NotFound => view("section").text("not found"),
		})
}

fun main() {
	let route = current_path().map(|path| parse(path));
	let _root = mount_root("app", || app(route));
}
"#;

/// The DOM/history stub plus the behavioral assertions, run under node against
/// the compiled bundle. Prints one `ok - ..` line per assertion; exits 1 on
/// any failure.
const HARNESS: &str = r#"class StubElement {
    constructor(tag) {
        this.tagName = tag;
        this.children = [];
        this.parent = null;
        this.listeners = {};
        this._text = "";
        this.className = "";
        this.hidden = false;
        this.value = "";
        this.attributes = {};
        this.style = { setProperty: () => {} };
    }
    set textContent(text) { this._text = text; this.children = []; }
    get textContent() { return this._text; }
    setAttribute(name, value) { this.attributes[name] = value; }
    appendChild(child) {
        if (child.parent) child.parent.children = child.parent.children.filter(c => c !== child);
        child.parent = this;
        this.children.push(child);
    }
    remove() {
        if (this.parent) {
            this.parent.children = this.parent.children.filter(c => c !== this);
            this.parent = null;
        }
    }
    replaceChildren() { for (const c of this.children) c.parent = null; this.children = []; }
    addEventListener(event, handler) { (this.listeners[event] = this.listeners[event] || []).push(handler); }
    click(overrides = {}) {
        const event = {
            button: 0, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false,
            prevented: false, preventDefault() { this.prevented = true; }, ...overrides,
        };
        for (const h of (this.listeners.click || [])) h(event);
        return event;
    }
    find(predicate) {
        if (predicate(this)) return this;
        for (const c of this.children) { const hit = c.find(predicate); if (hit) return hit; }
        return null;
    }
    render() {
        const kids = this.children.map(c => c.render()).join("");
        return `<${this.tagName}>${this._text}${kids}</${this.tagName}>`;
    }
}

const root = new StubElement("div");
global.document = {
    createElement: (tag) => new StubElement(tag),
    getElementById: (id) => (id === "app" ? root : null),
    querySelector: () => null,
    querySelectorAll: () => [],
};
global.location = { pathname: "/" };
global.history = {
    pushState(state, title, path) { global.location.pathname = path; },
};
const popstateHandlers = [];
global.window = { addEventListener: (ev, h) => { if (ev === "popstate") popstateHandlers.push(h); } };

require("./app.js");

let failures = 0;
const assert = (cond, msg) => {
    if (!cond) { failures += 1; console.error("FAIL - " + msg); }
    else console.log("ok   - " + msg);
};

const main = root.children[0];
const page = () => main.children[main.children.length - 1];

assert(page().tagName === "section" && page().textContent === "home", "initial route renders home");

const nav = main.children[0];
const homeLink = nav.children[0];
const tasksLink = nav.children[1];
assert(tasksLink.tagName === "a" && tasksLink.attributes.href === "/w/acme/tasks",
    "link renders <a href> printed from the route value");

let event = tasksLink.click();
assert(event.prevented, "plain left-click is intercepted (preventDefault)");
assert(global.location.pathname === "/w/acme/tasks", "link click navigated (pushState)");
assert(page().render().includes("acme") && page().render().includes("tasks"),
    "swap rendered the nested workspace layout");

const before = page();
tasksLink.click();
assert(page() === before, "navigating to the current route is a no-op (PartialEq dedupe)");

event = homeLink.click({ metaKey: true });
assert(!event.prevented && global.location.pathname === "/w/acme/tasks",
    "modified click keeps native anchor behavior (no interception)");

main.find(e => e.tagName === "button").click();
assert(global.location.pathname === "/login", "navigate() from an event handler");
const loginPage = page();
assert(loginPage.textContent === "/login", "page binding tracks current_path()");

homeLink.click();
assert(page().textContent === "home", "navigated back to home");
// Within the unmounting turn itself the page's subscriber may fire once more
// (notification order inside one drain — the recorded turn semantics); the
// disposal guarantee is about every LATER change.
const staleText = loginPage.textContent;
tasksLink.click();
assert(loginPage.textContent === staleText,
    "swapped-out subtree's subscription was disposed (detached element never updates again)");

global.location.pathname = "/login";
for (const h of popstateHandlers) h({});
assert(page().textContent === "/login", "popstate (back/forward) drives the same route signal");

process.exit(failures === 0 ? 0 : 1);
"#;

#[test]
fn router_swap_link_and_history_semantics() {
    let dir = temp_project("swap");
    write(
        &dir,
        "vilan.toml",
        "[package]\nname = \"router_e2e\"\nroot = \".\"\nentry = \"app.vl\"\ntarget = \"browser\"\n",
    );
    write(&dir, "app.vl", APP);
    write(&dir, "harness.js", HARNESS);

    let build = Command::new(env!("CARGO_BIN_EXE_vilan"))
        .args(["build", dir.to_str().unwrap()])
        .output()
        .expect("run vilan build");
    assert!(
        build.status.success(),
        "vilan build failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new("node")
        .arg("harness.js")
        .current_dir(&dir)
        .output()
        .expect("run node harness");
    assert!(
        run.status.success(),
        "router harness failed:\n{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
