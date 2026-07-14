function __json_kind(value) {
	if (value === null) return "null";
	if (Array.isArray(value)) return "array";
	return typeof value;
}
function __json_tag(value) {
	return typeof value === "string" ? value : Object.keys(value)[0];
}
function __try_parse_json(text) {
	try {
		return [ 0, JSON.parse(text) ];
	} catch (error) {
		return [ 1 ];
	}
}
function is_number(self) {
	return __json_kind(self) === "number";
}
function from_json_value(value) {
	let $m = null;
	if (is_number(value)) {
		$m = [ 0, Number(value) ];
	} else {
		$m = [ 1, "expected a number" ];
	}
	return $m;
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
	const $j = $g(__try_parse_json(text), "not valid JSON");
	if ($j[0] === 1) {
		return $j;
	}
	return from_json_value2($j[1]);
}
function from_json_value2(value) {
	const $k = __json_tag(value);
	let $l = null;
	if ($k === "Circle") {
		const $n = from_json_value(value["Circle"]);
		if ($n[0] === 1) {
			return $n;
		}
		$l = [ 0, [ 0, $n[1] ] ];
	} else if ($k === "Rect") {
		const $o = from_json_value(value["Rect"]["0"]);
		if ($o[0] === 1) {
			return $o;
		}
		const $p = from_json_value(value["Rect"]["1"]);
		if ($p[0] === 1) {
			return $p;
		}
		$l = [ 0, [ 1, $o[1], $p[1] ] ];
	} else if ($k === "Empty") {
		$l = [ 0, [ 2 ] ];
	} else {
		$l = [ 1, "unknown variant in JSON for enum Shape" ];
	}
	return $l;
}
function $g(self, err) {
	const $h = self;
	let $i = null;
	if ($h[0] === 0) {
		const x = $h[1];
		$i = [ 0, x ];
	} else {
		$i = [ 1, err ];
	}
	return $i;
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
const $q = from_json(to_json(c));
console.log($q[0] === 0 && eq($q[1], c));
const $r = from_json(to_json(r));
console.log($r[0] === 0 && eq($r[1], r));
const $s = from_json(to_json(e));
console.log($s[0] === 0 && eq($s[1], e));
const $t = from_json("\"Hexagon\"");
console.log($t[0] === 1);
console.log($t[1]);
