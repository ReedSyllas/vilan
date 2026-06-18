function aR/*default*/() {
	return 0;
}
function d(e, f) {
	const g = e;
	let h = null;
	if (g[0] === 0) {
		const i/*x*/ = g[1];
		h = [ 0, f(i/*x*/) ];
	} else {
		const j/*e*/ = g[1];
		h = [ 1, j/*e*/ ];
	}
	return h;
}
function k(l, m) {
	const n = l;
	let o = null;
	if (n[0] === 0) {
		const p/*x*/ = n[1];
		o = p/*x*/;
	} else {
		o = m;
	}
	return o;
}
function r(s, t) {
	const u = s;
	let v = null;
	if (u[0] === 0) {
		const w/*x*/ = u[1];
		v = [ 0, w/*x*/ ];
	} else {
		const x/*e*/ = u[1];
		v = [ 1, t(x/*e*/) ];
	}
	return v;
}
function y(l, m) {
	const z = l;
	let A = null;
	if (z[0] === 0) {
		const p/*x*/ = z[1];
		A = p/*x*/;
	} else {
		A = m;
	}
	return A;
}
function C(D, E) {
	const F = D;
	let G = null;
	if (F[0] === 0) {
		const H/*x*/ = F[1];
		G = E(H/*x*/);
	} else {
		G = false;
	}
	return G;
}
function J(K, L) {
	const M = K;
	let N = null;
	if (M[0] === 1) {
		const O/*e*/ = M[1];
		N = L(O/*e*/);
	} else {
		N = false;
	}
	return N;
}
function Q(R, S) {
	const T = R;
	let U = null;
	if (T[0] === 0) {
		const V/*x*/ = T[1];
		U = S(V/*x*/);
	} else {
		const W/*e*/ = T[1];
		U = [ 1, W/*e*/ ];
	}
	return U;
}
function X(l, m) {
	const Y = l;
	let Z = null;
	if (Y[0] === 0) {
		const p/*x*/ = Y[1];
		Z = p/*x*/;
	} else {
		Z = m;
	}
	return Z;
}
function ab(ac, ad) {
	const ae = ac;
	let af = null;
	if (ae[0] === 0) {
		const ag/*x*/ = ae[1];
		af = [ 0, ag/*x*/ ];
	} else {
		const ah/*e*/ = ae[1];
		af = ad(ah/*e*/);
	}
	return af;
}
function ai(l, m) {
	const aj = l;
	let ak = null;
	if (aj[0] === 0) {
		const p/*x*/ = aj[1];
		ak = p/*x*/;
	} else {
		ak = m;
	}
	return ak;
}
function am(an, ao) {
	const ap = an;
	let aq = null;
	if (ap[0] === 0) {
		const ar/*x*/ = ap[1];
		aq = ar/*x*/;
	} else {
		const as/*e*/ = ap[1];
		aq = ao(as/*e*/);
	}
	return aq;
}
function at(au) {
	const av = au;
	let aw = null;
	if (av[0] === 0) {
		const ax/*x*/ = av[1];
		aw = [ 0, ax/*x*/ ];
	} else {
		aw = [ 1 ];
	}
	return aw;
}
function ay(az) {
	const aA = az;
	return aA[0] === 0;
}
function aB(aC) {
	const aD = aC;
	let aE = null;
	if (aD[0] === 1) {
		const aF/*e*/ = aD[1];
		aE = [ 0, aF/*e*/ ];
	} else {
		aE = [ 1 ];
	}
	return aE;
}
function aG(aH, aI) {
	const aJ = aH;
	let aK = null;
	if (aJ[0] === 0) {
		const aL/*x*/ = aJ[1];
		aK = aL/*x*/;
	} else {
		aK = aI;
	}
	return aK;
}
function aM(aN) {
	const aO = aN;
	let aP = null;
	if (aO[0] === 0) {
		const aQ/*x*/ = aO[1];
		aP = aQ/*x*/;
	} else {
		aP = aR/*default*/();
	}
	return aP;
}
function aS(aT, aU) {
	const aV = aT;
	let aW = null;
	if (aV[0] === 0) {
		aW = aU;
	} else {
		const aX/*e*/ = aV[1];
		aW = [ 1, aX/*e*/ ];
	}
	return aW;
}
function aY(l, m) {
	const aZ = l;
	let ba = null;
	if (aZ[0] === 0) {
		const p/*x*/ = aZ[1];
		ba = p/*x*/;
	} else {
		ba = m;
	}
	return ba;
}
function bb(bc, bd) {
	const be = bc;
	let bf = null;
	if (be[0] === 0) {
		const bg/*x*/ = be[1];
		bf = [ 0, bg/*x*/ ];
	} else {
		bf = bd;
	}
	return bf;
}
function bh(l, m) {
	const bi = l;
	let bj = null;
	if (bi[0] === 0) {
		const p/*x*/ = bi[1];
		bj = p/*x*/;
	} else {
		bj = m;
	}
	return bj;
}
function bl(bm) {
	const bn = bm;
	let bo = null;
	if (bn[0] === 0 && bn[1][0] === 0) {
		const bp/*x*/ = bn[1][1];
		bo = [ 0, [ 0, bp/*x*/ ] ];
	} else if (bn[0] === 0 && bn[1][0] === 1) {
		bo = [ 1 ];
	} else {
		const bq/*e*/ = bn[1];
		bo = [ 0, [ 1, bq/*e*/ ] ];
	}
	return bo;
}
function br(az) {
	const bs = az;
	return bs[0] === 0;
}
const a/*ok*/ = [ 0, 10 ];
const b/*err*/ = [ 1, "boom" ];
console.log(k(d(a/*ok*/, (c) => {
	return c + 1;
}), 0));
console.log(y(r(b/*err*/, (q) => {
	return q;
}), 0));
console.log(C(a/*ok*/, (B) => {
	return B > 5;
}));
console.log(J(b/*err*/, (I) => {
	return true;
}));
console.log(X(Q(a/*ok*/, (P) => {
	return [ 0, P * 2 ];
}), 0));
console.log(ai(ab(b/*err*/, (aa) => {
	return [ 0, 7 ];
}), 0));
console.log(am(b/*err*/, (al) => {
	return 99;
}));
console.log(ay(at(a/*ok*/)));
console.log(aG(aB(b/*err*/), "none"));
console.log(aM(b/*err*/));
console.log(aY(aS(a/*ok*/, [ 0, 5 ]), 0));
console.log(bh(bb(b/*err*/, [ 0, 3 ]), 0));
const bk/*ro*/ = [ 0, [ 0, 42 ] ];
console.log(br(bl(bk/*ro*/)));
