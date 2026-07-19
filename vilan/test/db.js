import { DatabaseSync } from "node:sqlite";
function __db_all(statement, parameters) {
	return statement.all(...parameters);
}
function __db_close(database) {
	database.close();
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
function drop(self) {
	__db_close(self);
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
function $f($g) {
	drop($g);
}
let $a = undefined;
const db = new DatabaseSync(":memory:");
try {
	db.exec("CREATE TABLE task (id INTEGER PRIMARY KEY, title TEXT, done INTEGER)");
	const insert = db.prepare("INSERT INTO task (title, done) VALUES (?, ?)");
	run(insert, [ "write the pilot", 0 ]);
	run(insert, [ "ship std::db", 1 ]);
	const open_tasks = db.prepare("SELECT title FROM task WHERE done = ? ORDER BY id");
	for (const row of all(open_tasks, [ 0 ])) {
		console.log("todo: " + text(row, "title"));
	}
	const $b = first(db.prepare("SELECT COUNT(*) AS n FROM task"), [  ]);
	let $c = null;
	if ($b[0] === 0) {
		const row2 = $b[1];
		$c = console.log("" + integer(row2, "n") + " tasks total");
	} else {
		$c = console.log("no rows");
	}
	$c;
	db.exec("CREATE TABLE stamp (id INTEGER PRIMARY KEY, at INTEGER)");
	run(db.prepare("INSERT INTO stamp (at) VALUES (?)"), [ 1720656000000 ]);
	const $d = first(db.prepare("SELECT at FROM stamp"), [  ]);
	let $e = null;
	if ($d[0] === 0) {
		const row3 = $d[1];
		$e = console.log("stamp at " + big_integer(row3, "at"));
	} else {
		$e = console.log("no stamp");
	}
	$a = $e;
} finally {
	$f(db);
}
process.exit($a);
