function default2() {
	return 0;
}
function $a(self, fn) {
	const $b = self;
	let $c = null;
	if ($b[0] === 0) {
		const x = $b[1];
		$c = [ 0, fn(x) ];
	} else {
		const e = $b[1];
		$c = [ 1, e ];
	}
	return $c;
}
function $d(self2, fallback) {
	const $e = self2;
	let $f = null;
	if ($e[0] === 0) {
		const x2 = $e[1];
		$f = x2;
	} else {
		$f = fallback;
	}
	return $f;
}
function $g(self3, fn2) {
	const $h = self3;
	let $i = null;
	if ($h[0] === 0) {
		const x3 = $h[1];
		$i = [ 0, x3 ];
	} else {
		const e3 = $h[1];
		$i = [ 1, fn2(e3) ];
	}
	return $i;
}
function $j(self2, fallback) {
	const $k = self2;
	let $l = null;
	if ($k[0] === 0) {
		const x2 = $k[1];
		$l = x2;
	} else {
		$l = fallback;
	}
	return $l;
}
function $m(self4, fn3) {
	const $n = self4;
	let $o = null;
	if ($n[0] === 0) {
		const x4 = $n[1];
		$o = fn3(x4);
	} else {
		$o = false;
	}
	return $o;
}
function $p(self5, fn4) {
	const $q = self5;
	let $r = null;
	if ($q[0] === 1) {
		const e5 = $q[1];
		$r = fn4(e5);
	} else {
		$r = false;
	}
	return $r;
}
function $s(self6, fn5) {
	const $t = self6;
	let $u = null;
	if ($t[0] === 0) {
		const x5 = $t[1];
		$u = fn5(x5);
	} else {
		const e6 = $t[1];
		$u = [ 1, e6 ];
	}
	return $u;
}
function $v(self2, fallback) {
	const $w = self2;
	let $x = null;
	if ($w[0] === 0) {
		const x2 = $w[1];
		$x = x2;
	} else {
		$x = fallback;
	}
	return $x;
}
function $y(self7, fn6) {
	const $z = self7;
	let $A = null;
	if ($z[0] === 0) {
		const x6 = $z[1];
		$A = [ 0, x6 ];
	} else {
		const e8 = $z[1];
		$A = fn6(e8);
	}
	return $A;
}
function $B(self2, fallback) {
	const $C = self2;
	let $D = null;
	if ($C[0] === 0) {
		const x2 = $C[1];
		$D = x2;
	} else {
		$D = fallback;
	}
	return $D;
}
function $E(self8, fn7) {
	const $F = self8;
	let $G = null;
	if ($F[0] === 0) {
		const x7 = $F[1];
		$G = x7;
	} else {
		const e10 = $F[1];
		$G = fn7(e10);
	}
	return $G;
}
function $H(self9) {
	const $I = self9;
	let $J = null;
	if ($I[0] === 0) {
		const x8 = $I[1];
		$J = [ 0, x8 ];
	} else {
		$J = [ 1 ];
	}
	return $J;
}
function $K(self10) {
	const $L = self10;
	return $L[0] === 0;
}
function $M(self11) {
	const $N = self11;
	let $O = null;
	if ($N[0] === 1) {
		const e11 = $N[1];
		$O = [ 0, e11 ];
	} else {
		$O = [ 1 ];
	}
	return $O;
}
function $P(self12, fallback2) {
	const $Q = self12;
	let $R = null;
	if ($Q[0] === 0) {
		const x9 = $Q[1];
		$R = x9;
	} else {
		$R = fallback2;
	}
	return $R;
}
function $S(self13) {
	const $T = self13;
	let $U = null;
	if ($T[0] === 0) {
		const x10 = $T[1];
		$U = x10;
	} else {
		$U = default2();
	}
	return $U;
}
function $V(self14, b) {
	const $W = self14;
	let $X = null;
	if ($W[0] === 0) {
		$X = b;
	} else {
		const e12 = $W[1];
		$X = [ 1, e12 ];
	}
	return $X;
}
function $Y(self2, fallback) {
	const $Z = self2;
	let $aa = null;
	if ($Z[0] === 0) {
		const x2 = $Z[1];
		$aa = x2;
	} else {
		$aa = fallback;
	}
	return $aa;
}
function $ab(self15, b2) {
	const $ac = self15;
	let $ad = null;
	if ($ac[0] === 0) {
		const x11 = $ac[1];
		$ad = [ 0, x11 ];
	} else {
		$ad = b2;
	}
	return $ad;
}
function $ae(self2, fallback) {
	const $af = self2;
	let $ag = null;
	if ($af[0] === 0) {
		const x2 = $af[1];
		$ag = x2;
	} else {
		$ag = fallback;
	}
	return $ag;
}
function $ah(self16) {
	const $ai = self16;
	let $aj = null;
	if ($ai[0] === 0 && $ai[1][0] === 0) {
		const x12 = $ai[1][1];
		$aj = [ 0, [ 0, x12 ] ];
	} else if ($ai[0] === 0 && $ai[1][0] === 1) {
		$aj = [ 1 ];
	} else {
		const e13 = $ai[1];
		$aj = [ 0, [ 1, e13 ] ];
	}
	return $aj;
}
function $ak(self10) {
	const $al = self10;
	return $al[0] === 0;
}
const ok = [ 0, 10 ];
const err = [ 1, "boom" ];
console.log($d($a(ok, (n) => {
	return n + 1;
}), 0));
console.log($j($g(err, (e2) => {
	return e2;
}), 0));
console.log($m(ok, (n2) => {
	return n2 > 5;
}));
console.log($p(err, (e4) => {
	return true;
}));
console.log($v($s(ok, (n3) => {
	return [ 0, n3 * 2 ];
}), 0));
console.log($B($y(err, (e7) => {
	return [ 0, 7 ];
}), 0));
console.log($E(err, (e9) => {
	return 99;
}));
console.log($K($H(ok)));
console.log($P($M(err), "none"));
console.log($S(err));
console.log($Y($V(ok, [ 0, 5 ]), 0));
console.log($ae($ab(err, [ 0, 3 ]), 0));
const ro = [ 0, [ 0, 42 ] ];
console.log($ak($ah(ro)));
