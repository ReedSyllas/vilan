function __json_kind(value) {
	if (value === null) return "null";
	if (Array.isArray(value)) return "array";
	return typeof value;
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
function is_string(self) {
	return __json_kind(self) === "string";
}
function is_bool(self) {
	return __json_kind(self) === "boolean";
}
function has_field(self, name) {
	return Object.hasOwn(self, name);
}
function from_json_value(value) {
	let $i = null;
	if (is_string(value)) {
		$i = [ 0, String(value) ];
	} else {
		$i = [ 1, "expected a string" ];
	}
	return $i;
}
function from_json_value2(value) {
	let $k = null;
	if (is_number(value)) {
		$k = [ 0, Number(value) ];
	} else {
		$k = [ 1, "expected a number" ];
	}
	return $k;
}
function from_json_value3(value) {
	let $m = null;
	if (is_bool(value)) {
		$m = [ 0, Boolean(value) ];
	} else {
		$m = [ 1, "expected a boolean" ];
	}
	return $m;
}
function to_json(self) {
	return "{\"x\":" + JSON.stringify(self[0]) + "," + "\"y\":" + JSON.stringify(self[1]) + "}";
}
function from_json_value4(value) {
	let $o = null;
	if (!(has_field(value, "x"))) {
		return [ 1, "missing field x" ];
	}
	$o;
	let $p = null;
	if (!(has_field(value, "y"))) {
		return [ 1, "missing field y" ];
	}
	$p;
	const $q = from_json_value2(value["x"]);
	if ($q[0] === 1) {
		return $q;
	}
	const $r = from_json_value2(value["y"]);
	if ($r[0] === 1) {
		return $r;
	}
	return [ 0, [ $q[1], $r[1] ] ];
}
function to_json2(self) {
	return "{\"name\":" + JSON.stringify(self[0]) + "," + "\"age\":" + JSON.stringify(self[1]) + "," + "\"active\":" + JSON.stringify(self[2]) + "," + "\"home\":" + to_json(self[3]) + "}";
}
function from_json(text2) {
	const $d = $a(__try_parse_json(text2), "not valid JSON");
	if ($d[0] === 1) {
		return $d;
	}
	return from_json_value5($d[1]);
}
function from_json_value5(value) {
	let $e = null;
	if (!(has_field(value, "name"))) {
		return [ 1, "missing field name" ];
	}
	$e;
	let $f = null;
	if (!(has_field(value, "age"))) {
		return [ 1, "missing field age" ];
	}
	$f;
	let $g = null;
	if (!(has_field(value, "active"))) {
		return [ 1, "missing field active" ];
	}
	$g;
	let $h = null;
	if (!(has_field(value, "home"))) {
		return [ 1, "missing field home" ];
	}
	$h;
	const $j = from_json_value(value["name"]);
	if ($j[0] === 1) {
		return $j;
	}
	const $l = from_json_value2(value["age"]);
	if ($l[0] === 1) {
		return $l;
	}
	const $n = from_json_value3(value["active"]);
	if ($n[0] === 1) {
		return $n;
	}
	const $s = from_json_value4(value["home"]);
	if ($s[0] === 1) {
		return $s;
	}
	return [ 0, [ $j[1], $l[1], $n[1], $s[1] ] ];
}
function $a(self, err) {
	const $b = self;
	let $c = null;
	if ($b[0] === 0) {
		const x = $b[1];
		$c = [ 0, x ];
	} else {
		$c = [ 1, err ];
	}
	return $c;
}
const p = [ 1, 2 ];
console.log(to_json(p));
const person = [ "Ada \"A\"", 36, true, [ 3, 4 ] ];
const text = to_json2(person);
console.log(text);
const $t = from_json(text);
console.log($t[0] === 0 && to_json2($t[1]) === text && $t[1][3][1] === 4 && $t[1][0] === "Ada \"A\"");
const $u = from_json("{\"name\":\"x\",\"age\":1,\"active\":true}");
console.log($u[0] === 1);
console.log($u[1]);
const $v = from_json("{\"name\":5,\"age\":1,\"active\":true,\"home\":{\"x\":0,\"y\":0}}");
console.log($v[0] === 1);
console.log($v[1]);
