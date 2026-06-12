function b/*is_some*/(c) {
	const d = c;
	let e = null;
	if (d[0] === 0) {
		e = true;
	} else {
		e = false;
	}
	return e;
}
function f/*is_none*/(g) {
	const h = g;
	let i = null;
	if (h[0] === 0) {
		i = false;
	} else {
		i = true;
	}
	return i;
}
function m/*map*/(n, o) {
	const p = n;
	let q = null;
	if (p[0] === 0) {
		const r/*x*/ = p[1];
		q = [ 0, o(r/*x*/) ];
	} else {
		const s/*x*/ = p;
		q = s/*x*/;
	}
	return q;
}
const a/*v1*/ = [ 0, 1 ];
console.log(b/*is_some*/(a/*v1*/));
console.log(f/*is_none*/(a/*v1*/));
const j/*v2*/ = [ 1 ];
console.log(b/*is_some*/(j/*v2*/));
console.log(f/*is_none*/(j/*v2*/));
const k/*v3*/ = [ 0, 5 ];
console.log(m/*map*/(k/*v3*/, (l) => {
	return l * 10;
}));
const t/*v4*/ = [ 1 ];
console.log(m/*map*/(t/*v4*/, (u) => {
	return u * 10;
}));
