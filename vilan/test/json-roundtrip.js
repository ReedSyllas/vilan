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
function is_array(self) {
	return __json_kind(self) === "array";
}
function has_field(self, name) {
	return Object.hasOwn(self, name);
}
function from_json_value(value) {
	let $B = null;
	if (is_string(value)) {
		$B = [ 0, String(value) ];
	} else {
		$B = [ 1, "expected a string" ];
	}
	return $B;
}
function from_json_value2(value) {
	let $f = null;
	if (is_number(value)) {
		$f = [ 0, Number(value) ];
	} else {
		$f = [ 1, "expected a number" ];
	}
	return $f;
}
function to_json(self) {
	return "{\"name\":" + JSON.stringify(self[0]) + "," + "\"members\":" + $M(self[1]) + "," + "\"captain\":" + $N(self[2]) + "}";
}
function from_json(text) {
	const $x = $u(__try_parse_json(text), "not valid JSON");
	if ($x[0] === 1) {
		return $x;
	}
	return from_json_value3($x[1]);
}
function from_json_value3(value) {
	let $y = null;
	if (!(has_field(value, "name"))) {
		return [ 1, "missing field name" ];
	}
	$y;
	let $z = null;
	if (!(has_field(value, "members"))) {
		return [ 1, "missing field members" ];
	}
	$z;
	let $A = null;
	if (!(has_field(value, "captain"))) {
		return [ 1, "missing field captain" ];
	}
	$A;
	const $C = from_json_value(value["name"]);
	if ($C[0] === 1) {
		return $C;
	}
	const $G = $D(value["members"]);
	if ($G[0] === 1) {
		return $G;
	}
	const $K = $H(value["captain"]);
	if ($K[0] === 1) {
		return $K;
	}
	return [ 0, [ $C[1], $G[1], $K[1] ] ];
}
function $d(value) {
	let $e = null;
	if (!(is_array(value))) {
		return [ 1, "expected an array" ];
	}
	$e;
	let result = [  ];
	for (const element of value) {
		const $g = from_json_value2(element);
		if ($g[0] === 1) {
			return $g;
		}
		result.push($g[1]);
	}
	return [ 0, result ];
}
function $a(text) {
	const $b = __try_parse_json(text);
	let $c = null;
	if ($b[0] === 0) {
		const value = $b[1];
		$c = $d(value);
	} else {
		$c = [ 1, "not valid JSON" ];
	}
	return $c;
}
function $i(self) {
	let result = "[";
	let first = true;
	for (const element of self) {
		if (!(first)) {
			result = result + ",";
		}
		result = result + JSON.stringify(element);
		first = false;
	}
	return result + "]";
}
function $m(value) {
	let $n = null;
	if (value === null) {
		$n = [ 0, [ 1 ] ];
	} else {
		const $o = from_json_value2(value);
		if ($o[0] === 1) {
			return $o;
		}
		$n = [ 0, [ 0, $o[1] ] ];
	}
	return $n;
}
function $j(text) {
	const $k = __try_parse_json(text);
	let $l = null;
	if ($k[0] === 0) {
		const value = $k[1];
		$l = $m(value);
	} else {
		$l = [ 1, "not valid JSON" ];
	}
	return $l;
}
function $q(self) {
	const $r = self;
	let $s = null;
	if ($r[0] === 0) {
		const value = $r[1];
		$s = JSON.stringify(value);
	} else {
		$s = "null";
	}
	return $s;
}
function $u(self, err) {
	const $v = self;
	let $w = null;
	if ($v[0] === 0) {
		const x = $v[1];
		$w = [ 0, x ];
	} else {
		$w = [ 1, err ];
	}
	return $w;
}
function $D(value) {
	let $E = null;
	if (!(is_array(value))) {
		return [ 1, "expected an array" ];
	}
	$E;
	let result = [  ];
	for (const element of value) {
		const $F = from_json_value(element);
		if ($F[0] === 1) {
			return $F;
		}
		result.push($F[1]);
	}
	return [ 0, result ];
}
function $H(value) {
	let $I = null;
	if (value === null) {
		$I = [ 0, [ 1 ] ];
	} else {
		const $J = from_json_value(value);
		if ($J[0] === 1) {
			return $J;
		}
		$I = [ 0, [ 0, $J[1] ] ];
	}
	return $I;
}
function $M(self) {
	let result = "[";
	let first = true;
	for (const element of self) {
		if (!(first)) {
			result = result + ",";
		}
		result = result + JSON.stringify(element);
		first = false;
	}
	return result + "]";
}
function $N(self) {
	const $O = self;
	let $P = null;
	if ($O[0] === 0) {
		const value = $O[1];
		$P = JSON.stringify(value);
	} else {
		$P = "null";
	}
	return $P;
}
function $T(value) {
	let $U = null;
	if (!(is_array(value))) {
		return [ 1, "expected an array" ];
	}
	$U;
	let result = [  ];
	for (const element of value) {
		const $V = from_json_value3(element);
		if ($V[0] === 1) {
			return $V;
		}
		result.push($V[1]);
	}
	return [ 0, result ];
}
function $Q(text) {
	const $R = __try_parse_json(text);
	let $S = null;
	if ($R[0] === 0) {
		const value = $R[1];
		$S = $T(value);
	} else {
		$S = [ 1, "not valid JSON" ];
	}
	return $S;
}
function $X(self) {
	let result = "[";
	let first = true;
	for (const element of self) {
		if (!(first)) {
			result = result + ",";
		}
		result = result + to_json(element);
		first = false;
	}
	return result + "]";
}
const nums = $a("[1,2,3]");
const $h = nums;
console.log($h[0] === 0 && $i($h[1]) === "[1,2,3]");
const some = $j("7");
const $p = some;
console.log($p[0] === 0 && $q($p[1]) === "7");
const none = $j("null");
const $t = none;
console.log($t[0] === 0 && $q($t[1]) === "null");
const json = "{\"name\":\"Reds\",\"members\":[\"Ada\",\"Bob\"],\"captain\":\"Ada\"}";
const $L = from_json(json);
console.log($L[0] === 0 && to_json($L[1]) === json && $M($L[1][1]) === "[\"Ada\",\"Bob\"]");
const teams = $Q("[" + json + "]");
const $W = teams;
console.log($W[0] === 0 && $X($W[1]) === "[" + json + "]");
