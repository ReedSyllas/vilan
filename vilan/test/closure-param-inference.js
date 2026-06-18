function E/*fold*/(F, G, H) {
	let I/*accumulator*/ = G;
	for (const J/*item*/ of F) {
		I/*accumulator*/ = H(I/*accumulator*/, J/*item*/);
	}
	return I/*accumulator*/;
}
function c(d, e) {
	const f = d;
	let g = null;
	if (f[0] === 0) {
		const h/*x*/ = f[1];
		g = [ 0, e(h/*x*/) ];
	} else {
		g = [ 1 ];
	}
	return g;
}
function i(j, k) {
	const l = j;
	let m = null;
	if (l[0] === 0) {
		const n/*x*/ = l[1];
		m = n/*x*/;
	} else {
		m = k;
	}
	return m;
}
function p(q, r) {
	const s = q;
	let t = null;
	if (s[0] === 0) {
		const u/*x*/ = s[1];
		t = r(u/*x*/);
	} else {
		t = false;
	}
	return t;
}
function x(y, z) {
	let A/*result*/ = [  ];
	for (const B/*item*/ of y) {
		A/*result*/.push(z(B/*item*/));
	}
	return A/*result*/;
}
function L(M, N) {
	let O/*result*/ = [  ];
	for (const P/*item*/ of M) {
		if (N(P/*item*/)) {
			O/*result*/.push(P/*item*/);
		}
	}
	return O/*result*/;
}
const a/*p*/ = [ 0, [ 3, 4 ] ];
console.log(i(c(a/*p*/, (b) => {
	return b[0] + b[1];
}), 0));
console.log(p(a/*p*/, (o) => {
	return o[0] === 3;
}));
let v/*pts*/ = [  ];
v/*pts*/.push([ 1, 10 ]);
v/*pts*/.push([ 2, 20 ]);
console.log(E/*fold*/(x(v/*pts*/, (w) => {
	return w[0];
}), 0, (C, D) => {
	return C + D;
}));
console.log(L(v/*pts*/, (K) => {
	return K[1] > 15;
}).length);
