import { DatabaseSync } from "node:sqlite";
function __db_all(statement, parameters) {
	return statement.all(...parameters);
}
function __db_column(row, name) {
	return row[name];
}
function __db_get(statement, parameters) {
	const row = statement.get(...parameters);
	return row === undefined ? [ 1 ] : [ 0, row ];
}
function __db_run(statement, parameters) {
	const result = statement.run(...parameters);
	return Number(result.lastInsertRowid ?? 0);
}
function run(self, parameters) {
	return __db_run(self, parameters);
}
function all(self, parameters) {
	return __db_all(self, parameters);
}
function first(self, parameters) {
	return __db_get(self, parameters);
}
function text(self, name) {
	return __db_column(self, name);
}
function integer(self, name) {
	return __db_column(self, name);
}
function big_integer(self, name) {
	return __db_column(self, name);
}
const db = new DatabaseSync(":memory:");
db.exec("CREATE TABLE task (id INTEGER PRIMARY KEY, title TEXT, done INTEGER)");
const insert = db.prepare("INSERT INTO task (title, done) VALUES (?, ?)");
run(insert, [ "write the pilot", 0 ]);
run(insert, [ "ship std::db", 1 ]);
const open_tasks = db.prepare("SELECT title FROM task WHERE done = ? ORDER BY id");
for (const row of all(open_tasks, [ 0 ])) {
	console.log("todo: " + text(row, "title"));
}
const $a = first(db.prepare("SELECT COUNT(*) AS n FROM task"), [  ]);
let $b = null;
if ($a[0] === 0) {
	const row2 = $a[1];
	$b = console.log("" + integer(row2, "n") + " tasks total");
} else {
	$b = console.log("no rows");
}
$b;
db.exec("CREATE TABLE stamp (id INTEGER PRIMARY KEY, at INTEGER)");
run(db.prepare("INSERT INTO stamp (at) VALUES (?)"), [ 1720656000000 ]);
const $c = first(db.prepare("SELECT at FROM stamp"), [  ]);
let $d = null;
if ($c[0] === 0) {
	const row3 = $c[1];
	$d = console.log("stamp at " + big_integer(row3, "at"));
} else {
	$d = console.log("no stamp");
}
process.exit($d);
