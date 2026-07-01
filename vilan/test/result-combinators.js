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
function $j(self, fn) {
	const $k = self;
	let $l = null;
	if ($k[0] === 0) {
		const x = $k[1];
		$l = fn(x);
	} else {
		$l = false;
	}
	return $l;
}
function $m(self, fn) {
	const $n = self;
	let $o = null;
	if ($n[0] === 1) {
		const e = $n[1];
		$o = fn(e);
	} else {
		$o = false;
	}
	return $o;
}
function $p(self, fn) {
	const $q = self;
	let $r = null;
	if ($q[0] === 0) {
		const x = $q[1];
		$r = fn(x);
	} else {
		const e = $q[1];
		$r = [ 1, e ];
	}
	return $r;
}
function $s(self, fallback) {
	const $t = self;
	let $u = null;
	if ($t[0] === 0) {
		const x = $t[1];
		$u = x;
	} else {
		$u = fallback;
	}
	return $u;
}
function $v(self, fn) {
	const $w = self;
	let $x = null;
	if ($w[0] === 0) {
		const x = $w[1];
		$x = [ 0, x ];
	} else {
		const e = $w[1];
		$x = fn(e);
	}
	return $x;
}
function $y(self, fallback) {
	const $z = self;
	let $A = null;
	if ($z[0] === 0) {
		const x = $z[1];
		$A = x;
	} else {
		$A = fallback;
	}
	return $A;
}
function $B(self, fn) {
	const $C = self;
	let $D = null;
	if ($C[0] === 0) {
		const x = $C[1];
		$D = x;
	} else {
		const e = $C[1];
		$D = fn(e);
	}
	return $D;
}
function $E(self) {
	const $F = self;
	let $G = null;
	if ($F[0] === 0) {
		const x = $F[1];
		$G = [ 0, x ];
	} else {
		$G = [ 1 ];
	}
	return $G;
}
function $H(self) {
	const $I = self;
	return $I[0] === 0;
}
function $J(self) {
	const $K = self;
	let $L = null;
	if ($K[0] === 1) {
		const e = $K[1];
		$L = [ 0, e ];
	} else {
		$L = [ 1 ];
	}
	return $L;
}
function $M(self, fallback) {
	const $N = self;
	let $O = null;
	if ($N[0] === 0) {
		const x = $N[1];
		$O = x;
	} else {
		$O = fallback;
	}
	return $O;
}
function $P(self) {
	const $Q = self;
	let $R = null;
	if ($Q[0] === 0) {
		const x = $Q[1];
		$R = x;
	} else {
		$R = default2();
	}
	return $R;
}
function $S(self, b) {
	const $T = self;
	let $U = null;
	if ($T[0] === 0) {
		$U = b;
	} else {
		const e = $T[1];
		$U = [ 1, e ];
	}
	return $U;
}
function $V(self, b) {
	const $W = self;
	let $X = null;
	if ($W[0] === 0) {
		const x = $W[1];
		$X = [ 0, x ];
	} else {
		$X = b;
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
function $ab(self) {
	const $ac = self;
	let $ad = null;
	if ($ac[0] === 0 && $ac[1][0] === 0) {
		const x = $ac[1][1];
		$ad = [ 0, [ 0, x ] ];
	} else if ($ac[0] === 0 && $ac[1][0] === 1) {
		$ad = [ 1 ];
	} else {
		const e = $ac[1];
		$ad = [ 0, [ 1, e ] ];
	}
	return $ad;
}
function $ae(self) {
	const $af = self;
	return $af[0] === 0;
}
const ok = [ 0, 10 ];
const err = [ 1, "boom" ];
console.log($d($a(ok, (n) => {
	return n + 1;
}), 0));
console.log($d($g(err, (e) => {
	return e;
}), 0));
console.log($j(ok, (n) => {
	return n > 5;
}));
console.log($m(err, (e) => {
	return true;
}));
console.log($s($p(ok, (n) => {
	return [ 0, n * 2 ];
}), 0));
console.log($y($v(err, (e) => {
	return [ 0, 7 ];
}), 0));
console.log($B(err, (e) => {
	return 99;
}));
console.log($H($E(ok)));
console.log($M($J(err), "none"));
console.log($P(err));
console.log($d($S(ok, [ 0, 5 ]), 0));
console.log($Y($V(err, [ 0, 3 ]), 0));
const ro = [ 0, [ 0, 42 ] ];
console.log($ae($ab(ro)));
