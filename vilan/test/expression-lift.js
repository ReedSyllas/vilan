function __at(list, index) {
	if (index >= 0 && index < list.length) return list[index];
	throw "index out of bounds: the length is " + list.length + " but the index is " + index;
}
function fetch2(log3, value) {
	log3.push(1);
	return value;
}
function parse(tag) {
	let $o = null;
	if (tag === "good") {
		$o = [ 0, 21 ];
	} else {
		$o = [ 1, "bad: " + tag ];
	}
	return $o;
}
function total(a, b) {
	let $A = null;
	const $B = a;
	if ($B[0] === 1) {
		$A = $B;
	} else {
		const $C = b;
		if ($C[0] === 1) {
			$A = $C;
		} else {
			$A = [ 0, $B[1] + $C[1] ];
		}
	}
	const $D = $A;
	if ($D[0] === 1) {
		return $D;
	}
	const sum2 = $D[1];
	return [ 0, sum2 * 10 ];
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
const count = [ 0, 2 ];
let $a = null;
const $b = count;
if ($b[0] === 1) {
	$a = $b;
} else {
	$a = [ 0, $b[1] * 2 ];
}
console.log($c($a, -(1)));
let $f = null;
const $g = count;
if ($g[0] === 1) {
	$f = $g;
} else {
	$f = [ 0, 2 * $g[1] ];
}
console.log($c($f, -(1)));
let log = [  ];
let $h = null;
const $i = fetch2(log, [ 0, 40 ]);
if ($i[0] === 1) {
	$h = $i;
} else {
	const $j = fetch2(log, [ 0, 2 ]);
	if ($j[0] === 1) {
		$h = $j;
	} else {
		$h = [ 0, $i[1] + $j[1] ];
	}
}
const both = $h;
console.log($c(both, -(1)));
console.log(log.length);
let log2 = [  ];
let $k = null;
const $l = fetch2(log2, [ 1 ]);
if ($l[0] === 1) {
	$k = $l;
} else {
	const $m = fetch2(log2, [ 0, 2 ]);
	if ($m[0] === 1) {
		$k = $m;
	} else {
		$k = [ 0, $l[1] + $m[1] ];
	}
}
const bad = $k;
console.log($c(bad, -(1)));
console.log(log2.length);
let $n = null;
const $p = parse("good");
if ($p[0] === 1) {
	$n = $p;
} else {
	const $q = parse("good");
	if ($q[0] === 1) {
		$n = $q;
	} else {
		$n = [ 0, $p[1] + $q[1] ];
	}
}
const sum = $n;
const $r = sum;
let $s = null;
if ($r[0] === 0) {
	const n = $r[1];
	$s = console.log(n);
} else {
	const e = $r[1];
	$s = console.log(e);
}
$s;
let $t = null;
const $u = parse("x");
if ($u[0] === 1) {
	$t = $u;
} else {
	const $v = parse("y");
	if ($v[0] === 1) {
		$t = $v;
	} else {
		$t = [ 0, $u[1] + $v[1] ];
	}
}
const $w = $t;
let $x = null;
if ($w[0] === 0) {
	const n2 = $w[1];
	$x = console.log(n2);
} else {
	const e2 = $w[1];
	$x = console.log(e2);
}
$x;
const rows = [ 0, [ [ 0, 7 ], [ 1 ] ] ];
let $y = null;
const $z = rows;
if ($z[0] === 1) {
	$y = $z;
} else {
	$y = __at($z[1], 0);
}
const first = $y;
console.log($c(first, -(1)));
console.log($c(total([ 0, 4 ], [ 0, 2 ]), -(1)));
console.log($c(total([ 0, 4 ], [ 1 ]), -(1)));
const size = [ 0, 4 ];
let $E = null;
const $F = size;
if ($F[0] === 1) {
	$E = $F;
} else {
	const $G = size;
	if ($G[0] === 1) {
		$E = $G;
	} else {
		$E = [ 0, $F[1] * $G[1] ];
	}
}
console.log($c($E, -(1)));
