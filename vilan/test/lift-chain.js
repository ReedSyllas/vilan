function __parse_i32(text) {
	const trimmed = text.trim();
	const value = Number(trimmed);
	return /^[+-]?[0-9]+$/.test(trimmed) && value >= -2147483648 && value <= 2147483647 ? [ 0, value ] : [ 1 ];
}
function shelf(self) {
	let $p = null;
	if (self[0] === "dune") {
		$p = [ 0, "sci-fi" ];
	} else {
		$p = [ 1 ];
	}
	return $p;
}
function find(key) {
	let $a = null;
	if (key === "hit") {
		$a = [ 0, [ "dune", "messiah" ] ];
	} else {
		$a = [ 1 ];
	}
	return $a;
}
function to_number(text) {
	const $s = __parse_i32(text);
	let $t = null;
	if ($s[0] === 0) {
		const value = $s[1];
		$t = [ 0, value ];
	} else {
		$t = [ 1, text ];
	}
	return $t;
}
function headline(key) {
	const $C = find(key);
	let $D = null;
	if ($C[0] === 1) {
		$D = $C;
	} else {
		$D = [ 0, $C[1][0] ];
	}
	const $E = $D;
	if ($E[0] === 1) {
		return $E;
	}
	const title = $E[1];
	return [ 0, title.toUpperCase() ];
}
function $d(self, fallback) {
	const $e = self;
	let $f = null;
	if ($e[0] === 0) {
		const x = $e[1];
		$f = x;
	} else {
		$f = fallback;
	}
	return $f;
}
function $i(self, fallback) {
	const $j = self;
	let $k = null;
	if ($j[0] === 0) {
		const x = $j[1];
		$k = x;
	} else {
		$k = fallback;
	}
	return $k;
}
const $b = find("hit");
let $c = null;
if ($b[0] === 1) {
	$c = $b;
} else {
	$c = [ 0, $b[1][0] ];
}
console.log($d($c, "?"));
const $g = find("hit");
let $h = null;
if ($g[0] === 1) {
	$h = $g;
} else {
	$h = [ 0, $g[1][1].length ];
}
console.log($i($h, 0 - 1));
const $l = find("miss");
let $m = null;
if ($l[0] === 1) {
	$m = $l;
} else {
	$m = [ 0, $l[1][0] ];
}
console.log($d($m, "?"));
const $n = find("hit");
let $o = null;
if ($n[0] === 1) {
	$o = $n;
} else {
	$o = shelf($n[1]);
}
console.log($d($o, "?"));
const $q = find("miss");
let $r = null;
if ($q[0] === 1) {
	$r = $q;
} else {
	$r = shelf($q[1]);
}
console.log($d($r, "?"));
const $u = to_number("40");
let $v = null;
if ($u[0] === 1) {
	$v = $u;
} else {
	$v = [ 0, Math.max($u[1], 2) ];
}
const $w = $v;
let $x = null;
if ($w[0] === 0) {
	const v = $w[1];
	$x = console.log(v);
} else {
	const e = $w[1];
	$x = console.log(e);
}
$x;
const $y = to_number("nope");
let $z = null;
if ($y[0] === 1) {
	$z = $y;
} else {
	$z = [ 0, Math.max($y[1], 2) ];
}
const $A = $z;
let $B = null;
if ($A[0] === 0) {
	const v2 = $A[1];
	$B = console.log(v2);
} else {
	const e2 = $A[1];
	$B = console.log(e2);
}
$B;
console.log($d(headline("hit"), "?"));
console.log($d(headline("miss"), "?"));
