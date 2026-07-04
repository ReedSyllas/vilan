function __parse_i32(text) {
	const trimmed = text.trim();
	const value = Number(trimmed);
	return /^[+-]?[0-9]+$/.test(trimmed) && value >= -2147483648 && value <= 2147483647 ? [ 0, value ] : [ 1 ];
}
function lookup(key) {
	let $a = null;
	if (key === "hit") {
		$a = [ 0, 21 ];
	} else {
		$a = [ 1 ];
	}
	return $a;
}
function doubled(key) {
	const $b = lookup(key);
	if ($b[0] === 1) {
		return $b;
	}
	const value = $b[1];
	return [ 0, value * 2 ];
}
function to_number(text) {
	const $f = __parse_i32(text);
	let $g = null;
	if ($f[0] === 0) {
		const value = $f[1];
		$g = [ 0, value ];
	} else {
		$g = [ 1, text ];
	}
	return $g;
}
function sum(a2, b2) {
	const $h = to_number(a2);
	if ($h[0] === 1) {
		return $h;
	}
	const left = $h[1];
	const $i = to_number(b2);
	if ($i[0] === 1) {
		return $i;
	}
	const right = $i[1];
	return [ 0, left + right ];
}
function verdict(self) {
	const $o = self;
	let $p = null;
	if ($o[0] === 0) {
		const lane3 = $o[1];
		$p = [ 0, lane3 ];
	} else {
		const why3 = $o[1];
		$p = [ 1, why3 ];
	}
	return $p;
}
function from_bad(bad) {
	return [ 1, bad ];
}
function pass(gate) {
	const $n = gate;
	const $q = verdict($n);
	if ($q[0] === 1) {
		return from_bad($q[1]);
	}
	const lane3 = $q[1];
	return [ 0, lane3 + 1 ];
}
function is_twenty_one() {
	const $v = lookup("hit");
	if ($v[0] === 1) {
		return $v;
	}
	return [ 0, $v[1] === 21 ];
}
function $c(self, fallback) {
	const $d = self;
	let $e = null;
	if ($d[0] === 0) {
		const x = $d[1];
		$e = x;
	} else {
		$e = fallback;
	}
	return $e;
}
function $w(self, fallback) {
	const $x = self;
	let $y = null;
	if ($x[0] === 0) {
		const x = $x[1];
		$y = x;
	} else {
		$y = fallback;
	}
	return $y;
}
console.log($c(doubled("hit"), 0 - 1));
console.log($c(doubled("miss"), 0 - 1));
const $j = sum("40", "2");
let $k = null;
if ($j[0] === 0) {
	const v = $j[1];
	$k = console.log(v);
} else {
	const e = $j[1];
	$k = console.log(e);
}
$k;
const $l = sum("40", "two");
let $m = null;
if ($l[0] === 0) {
	const v2 = $l[1];
	$m = console.log(v2);
} else {
	const e2 = $l[1];
	$m = console.log(e2);
}
$m;
const $r = pass([ 0, 6 ]);
let $s = null;
if ($r[0] === 0) {
	const lane = $r[1];
	$s = console.log(lane);
} else {
	const why = $r[1];
	$s = console.log(why);
}
$s;
const $t = pass([ 1, "closed" ]);
let $u = null;
if ($t[0] === 0) {
	const lane2 = $t[1];
	$u = console.log(lane2);
} else {
	const why2 = $t[1];
	$u = console.log(why2);
}
$u;
const a = 1;
const b = 2;
console.log(a !== b);
console.log($w(is_twenty_one(), false));
