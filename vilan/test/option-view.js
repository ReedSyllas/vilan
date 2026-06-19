function b/*get_mut*/(c) {
	return [ 0, [ c, 0 ] ];
}
function g/*get*/(h) {
	return [ 0, [ h, 0 ] ];
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
process.exit(j);
