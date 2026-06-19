function b/*get_mut*/(c) {
	return [ 0, [ c, 0 ] ];
}
function g/*get*/(h) {
	return [ 0, [ h, 0 ] ];
}
function m/*inner_mut*/(n) {
	return [ 0, n[0] ];
}
function r/*item_mut*/(s, t) {
	let u = null;
	if (t < s[1].length) {
		u = [ 0, [ s[1], t ] ];
	} else {
		u = [ 1 ];
	}
	return u;
}
let a/*slot*/ = [ 1 ];
const d = b/*get_mut*/(a/*slot*/);
let e = null;
if (d[0] === 0) {
	const f/*v*/ = d[1];
	f/*v*/[0][f/*v*/[1]] = 42;
	e = undefined;
} else {
	e = undefined;
}
e;
console.log(a/*slot*/[0]);
const i = g/*get*/(a/*slot*/);
let j = null;
if (i[0] === 0) {
	const k/*v*/ = i[1];
	console.log(k/*v*/[0][k/*v*/[1]]);
	j = undefined;
} else {
	j = undefined;
}
j;
let l/*outer*/ = [ [ 1 ], [ 10, 20, 30 ] ];
const o = m/*inner_mut*/(l/*outer*/);
let p = null;
if (o[0] === 0) {
	const q/*v*/ = o[1];
	q/*v*/[0] = 77;
	p = undefined;
} else {
	p = undefined;
}
p;
console.log(l/*outer*/[0][0]);
const v = r/*item_mut*/(l/*outer*/, 1);
let w = null;
if (v[0] === 0) {
	const x/*v*/ = v[1];
	x/*v*/[0][x/*v*/[1]] = 99;
	w = undefined;
} else {
	w = undefined;
}
w;
console.log(l/*outer*/[1][1]);
const y = r/*item_mut*/(l/*outer*/, 9);
let z = null;
if (y[0] === 0) {
	const A/*v*/ = y[1];
	A/*v*/[0][A/*v*/[1]] = 0;
	z = undefined;
} else {
	console.log(0);
	z = undefined;
}
process.exit(z);
