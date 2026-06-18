function g/*eq*/(h, i) {
	return h[0] === i[0] && h[1] === i[1];
}
function d(e, f) {
	return g/*eq*/(e, f);
}
function j(k, l) {
	return g/*eq*/(k, l);
}
function m(n, o) {
	return !(g/*eq*/(n, o));
}
function p(e, f) {
	return e === f;
}
function q(n, o) {
	return n !== o;
}
function u(v, w) {
	const x = [ v, w ];
	let y = null;
	if (x[0][0] === 0 && x[1][0] === 0) {
		const z/*x*/ = x[0][1];
		const A/*y*/ = x[1][1];
		y = g/*eq*/(z/*x*/, A/*y*/);
	} else if (x[0][0] === 1 && x[1][0] === 1) {
		y = true;
	} else {
		y = false;
	}
	return y;
}
const a/*p1*/ = [ 1, 2 ];
const b/*p2*/ = [ 1, 2 ];
const c/*p3*/ = [ 3, 4 ];
console.log(d(a/*p1*/, b/*p2*/));
console.log(d(a/*p1*/, c/*p3*/));
console.log(j(a/*p1*/, b/*p2*/));
console.log(m(a/*p1*/, c/*p3*/));
console.log(p(5, 5));
console.log(p(5, 9));
console.log(q(5, 9));
const r/*some_a*/ = [ 0, a/*p1*/ ];
const s/*some_b*/ = [ 0, b/*p2*/ ];
const t/*some_c*/ = [ 0, c/*p3*/ ];
console.log(u(r/*some_a*/, s/*some_b*/));
console.log(u(r/*some_a*/, t/*some_c*/));
console.log(!(u(r/*some_a*/, t/*some_c*/)));
console.log(u(r/*some_a*/, [ 1 ]));
