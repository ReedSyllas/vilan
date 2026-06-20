function __json_tag(value) {
	return typeof value === "string" ? value : Object.keys(value)[0];
}
function eq(self, other) {
	const $a = [ self, other ];
	let $b = null;
	if ($a[0][0] === 0 && $a[1][0] === 0) {
		const s0 = $a[0][1];
		const o0 = $a[1][1];
		$b = s0 === o0;
	} else if ($a[0][0] === 1 && $a[1][0] === 1) {
		const s02 = $a[0][1];
		const s1 = $a[0][2];
		const o02 = $a[1][1];
		const o1 = $a[1][2];
		$b = s02 === o02 && s1 === o1;
	} else if ($a[0][0] === 2 && $a[1][0] === 2) {
		$b = true;
	} else {
		$b = false;
	}
	return $b;
}
function debug(self) {
	const $c = self;
	let $d = null;
	if ($c[0] === 0) {
		const p0 = $c[1];
		$d = "Circle(" + JSON.stringify(p0) + ")";
	} else if ($c[0] === 1) {
		const p02 = $c[1];
		const p1 = $c[2];
		$d = "Rect(" + JSON.stringify(p02) + ", " + JSON.stringify(p1) + ")";
	} else {
		$d = "Empty";
	}
	return $d;
}
function to_json(self) {
	const $e = self;
	let $f = null;
	if ($e[0] === 0) {
		const p0 = $e[1];
		$f = "{\"Circle\":" + JSON.stringify(p0) + "}";
	} else if ($e[0] === 1) {
		const p02 = $e[1];
		const p1 = $e[2];
		$f = "{\"Rect\":[" + JSON.stringify(p02) + "," + JSON.stringify(p1) + "]}";
	} else {
		$f = "\"Empty\"";
	}
	return $f;
}
function from_json(text) {
	return from_json_value(JSON.parse(text));
}
function from_json_value(value) {
	const $g = __json_tag(value);
	let $h = null;
	if ($g === "Circle") {
		$h = [ 0, Number(value["Circle"]) ];
	} else if ($g === "Rect") {
		$h = [ 1, Number(value["Rect"]["0"]), Number(value["Rect"]["1"]) ];
	} else if ($g === "Empty") {
		$h = [ 2 ];
	} else {
		$h = (() => {
			throw "unknown variant in JSON for enum Shape";
		})();
	}
	return $h;
}
const c = [ 0, 3 ];
const r = [ 1, 4, 5 ];
const e = [ 2 ];
console.log(eq(c, [ 0, 3 ]));
console.log(eq(c, [ 0, 9 ]));
console.log(eq(c, r));
console.log(eq(e, [ 2 ]));
console.log(debug(c));
console.log(debug(r));
console.log(debug(e));
console.log(to_json(c));
console.log(to_json(r));
console.log(to_json(e));
console.log(eq(from_json(to_json(c)), c));
console.log(eq(from_json(to_json(r)), r));
console.log(eq(from_json(to_json(e)), e));
