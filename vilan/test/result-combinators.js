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
function $g(self, fn) {
	const $h = self;
	let $i = null;
	if ($h[0] === 0) {
		const x = $h[1];
		$i = [ 0, x ];
	} else {
		const e = $h[1];
		$i = [ 1, fn(e) ];
	}
	return $i;
}
function $j(self, fallback) {
	const $k = self;
	let $l = null;
	if ($k[0] === 0) {
		const x = $k[1];
		$l = x;
	} else {
		$l = fallback;
	}
	return $l;
}
function $m(self, fn) {
	const $n = self;
	let $o = null;
	if ($n[0] === 0) {
		const x = $n[1];
		$o = fn(x);
	} else {
		$o = false;
	}
	return $o;
}
function $p(self, fn) {
	const $q = self;
	let $r = null;
	if ($q[0] === 1) {
		const e = $q[1];
		$r = fn(e);
	} else {
		$r = false;
	}
	return $r;
}
function $s(self, fn) {
	const $t = self;
	let $u = null;
	if ($t[0] === 0) {
		const x = $t[1];
		$u = fn(x);
	} else {
		const e = $t[1];
		$u = [ 1, e ];
	}
	return $u;
}
function $v(self, fallback) {
	const $w = self;
	let $x = null;
	if ($w[0] === 0) {
		const x = $w[1];
		$x = x;
	} else {
		$x = fallback;
	}
	return $x;
}
function $y(self, fn) {
	const $z = self;
	let $A = null;
	if ($z[0] === 0) {
		const x = $z[1];
		$A = [ 0, x ];
	} else {
		const e = $z[1];
		$A = fn(e);
	}
	return $A;
}
function $B(self, fallback) {
	const $C = self;
	let $D = null;
	if ($C[0] === 0) {
		const x = $C[1];
		$D = x;
	} else {
		$D = fallback;
	}
	return $D;
}
function $E(self, fn) {
	const $F = self;
	let $G = null;
	if ($F[0] === 0) {
		const x = $F[1];
		$G = x;
	} else {
		const e = $F[1];
		$G = fn(e);
	}
	return $G;
}
function $H(self) {
	const $I = self;
	let $J = null;
	if ($I[0] === 0) {
		const x = $I[1];
		$J = [ 0, x ];
	} else {
		$J = [ 1 ];
	}
	return $J;
}
function $K(self) {
	const $L = self;
	return $L[0] === 0;
}
function $M(self) {
	const $N = self;
	let $O = null;
	if ($N[0] === 1) {
		const e = $N[1];
		$O = [ 0, e ];
	} else {
		$O = [ 1 ];
	}
	return $O;
}
function $P(self, fallback) {
	const $Q = self;
	let $R = null;
	if ($Q[0] === 0) {
		const x = $Q[1];
		$R = x;
	} else {
		$R = fallback;
	}
	return $R;
}
function $S(self) {
	const $T = self;
	let $U = null;
	if ($T[0] === 0) {
		const x = $T[1];
		$U = x;
	} else {
		$U = default2();
	}
	return $U;
}
function $V(self, b) {
	const $W = self;
	let $X = null;
	if ($W[0] === 0) {
		$X = b;
	} else {
		const e = $W[1];
		$X = [ 1, e ];
	}
	return $X;
}
function $Y(self, fallback) {
	const $Z = self;
	let $aa = null;
	if ($Z[0] === 0) {
		const x = $Z[1];
		$aa = x;
	} else {
		$aa = fallback;
	}
	return $aa;
}
function $ab(self, b) {
	const $ac = self;
	let $ad = null;
	if ($ac[0] === 0) {
		const x = $ac[1];
		$ad = [ 0, x ];
	} else {
		$ad = b;
	}
	return $ad;
}
function $ae(self, fallback) {
	const $af = self;
	let $ag = null;
	if ($af[0] === 0) {
		const x = $af[1];
		$ag = x;
	} else {
		$ag = fallback;
	}
	return $ag;
}
function $ah(self) {
	const $ai = self;
	let $aj = null;
	if ($ai[0] === 0 && $ai[1][0] === 0) {
		const x = $ai[1][1];
		$aj = [ 0, [ 0, x ] ];
	} else if ($ai[0] === 0 && $ai[1][0] === 1) {
		$aj = [ 1 ];
	} else {
		const e = $ai[1];
		$aj = [ 0, [ 1, e ] ];
	}
	return $aj;
}
function $ak(self) {
	const $al = self;
	return $al[0] === 0;
}
const ok = [ 0, 10 ];
const err = [ 1, "boom" ];
console.log($d($a(ok, (n) => {
	return n + 1;
}), 0));
console.log($j($g(err, (e) => {
	return e;
}), 0));
console.log($m(ok, (n) => {
	return n > 5;
}));
console.log($p(err, (e) => {
	return true;
}));
console.log($v($s(ok, (n) => {
	return [ 0, n * 2 ];
}), 0));
console.log($B($y(err, (e) => {
	return [ 0, 7 ];
}), 0));
console.log($E(err, (e) => {
	return 99;
}));
console.log($K($H(ok)));
console.log($P($M(err), "none"));
console.log($S(err));
console.log($Y($V(ok, [ 0, 5 ]), 0));
console.log($ae($ab(err, [ 0, 3 ]), 0));
const ro = [ 0, [ 0, 42 ] ];
console.log($ak($ah(ro)));
