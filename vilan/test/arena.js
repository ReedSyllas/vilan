function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
function b/*new*/() {
	return [ [  ], [  ] ];
}
function Y/*sum_from*/(Z, aa) {
	const ae = ab(Z, aa);
	let af = null;
	if (ae[0] === 0) {
		const ag/*node*/ = ae[1];
		let ah/*total*/ = ag/*node*/[0];
		for (const ai/*edge*/ of ag/*node*/[1]) {
			ah/*total*/ = ah/*total*/ + Y/*sum_from*/(Z, ai/*edge*/);
		}
		af = ah/*total*/;
	} else {
		af = 0;
	}
	return af;
}
function d(e, f) {
	const g = __list_pop(e[1]);
	let h = null;
	if (g[0] === 0) {
		const i/*index*/ = g[1];
		e[0][i/*index*/][1] = [ 0, f ];
		h = [ i/*index*/, e[0][i/*index*/][0] ];
	} else {
		const j/*index*/ = e[0].length;
		e[0].push([ 0, [ 0, f ] ]);
		h = [ j/*index*/, 0 ];
	}
	return h;
}
function l(m) {
	return m[0].length - m[1].length;
}
function t(u) {
	const v = u;
	return v[0] === 0;
}
function q(r, s) {
	return s[0] < r[0].length && r[0][s[0]][0] === s[1] && t(r[0][s[0]][1]);
}
function n(o, p) {
	let w = null;
	if (q(o, p)) {
		w = o[0][p[0]][1];
	} else {
		w = [ 1 ];
	}
	return w;
}
function x(y, z) {
	const A = y;
	let B = null;
	if (A[0] === 0) {
		const C/*x*/ = A[1];
		B = C/*x*/;
	} else {
		B = z;
	}
	return B;
}
function D(E, F, G) {
	let H = null;
	if (q(E, F)) {
		E[0][F[0]][1] = [ 0, G ];
		H = true;
	} else {
		H = false;
	}
	return H;
}
function I(J, K) {
	let M = null;
	if (q(J, K)) {
		const L/*removed*/ = J[0][K[0]][1];
		J[0][K[0]][0] = J[0][K[0]][0] + 1;
		J[0][K[0]][1] = [ 1 ];
		J[1].push(K[0]);
		M = L/*removed*/;
	} else {
		M = [ 1 ];
	}
	return M;
}
function N(u) {
	const O = u;
	return O[0] === 0;
}
function S(e, f) {
	const T = __list_pop(e[1]);
	let U = null;
	if (T[0] === 0) {
		const i/*index*/ = T[1];
		e[0][i/*index*/][1] = [ 0, f ];
		U = [ i/*index*/, e[0][i/*index*/][0] ];
	} else {
		const j/*index*/ = e[0].length;
		e[0].push([ 0, [ 0, f ] ]);
		U = [ j/*index*/, 0 ];
	}
	return U;
}
function ac(r, s) {
	return s[0] < r[0].length && r[0][s[0]][0] === s[1] && t(r[0][s[0]][1]);
}
function ab(o, p) {
	let ad = null;
	if (ac(o, p)) {
		ad = o[0][p[0]][1];
	} else {
		ad = [ 1 ];
	}
	return ad;
}
let a/*numbers*/ = b/*new*/();
const c/*a*/ = d(a/*numbers*/, 10);
const k/*b*/ = d(a/*numbers*/, 20);
console.log(l(a/*numbers*/));
console.log(x(n(a/*numbers*/, c/*a*/), -(1)));
D(a/*numbers*/, k/*b*/, 99);
console.log(x(n(a/*numbers*/, k/*b*/), -(1)));
console.log(x(I(a/*numbers*/, k/*b*/), -(1)));
console.log(N(n(a/*numbers*/, k/*b*/)));
const P/*c*/ = d(a/*numbers*/, 30);
console.log(x(n(a/*numbers*/, P/*c*/), -(1)));
console.log(N(n(a/*numbers*/, k/*b*/)));
console.log(x(n(a/*numbers*/, c/*a*/), -(1)));
let Q/*graph*/ = b/*new*/();
const R/*leaf1*/ = S(Q/*graph*/, [ 2, [  ] ]);
const V/*leaf2*/ = S(Q/*graph*/, [ 3, [  ] ]);
let W/*root_edges*/ = [  ];
W/*root_edges*/.push(R/*leaf1*/);
W/*root_edges*/.push(V/*leaf2*/);
const X/*root*/ = S(Q/*graph*/, [ 1, W/*root_edges*/ ]);
console.log(Y/*sum_from*/(Q/*graph*/, X/*root*/));
