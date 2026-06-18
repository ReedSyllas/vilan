function __parse_f64(text) {
	const value = Number.parseFloat(text);
	return Number.isNaN(value) ? [ 1 ] : [ 0, value ];
}
function a(b, c) {
	const d = b;
	let e = null;
	if (d[0] === 0) {
		const f/*x*/ = d[1];
		e = f/*x*/;
	} else {
		e = c;
	}
	return e;
}
function g(h) {
	const i = h;
	return i[0] === 0;
}
console.log(a(__parse_f64("3.14"), 0));
console.log(a(__parse_f64("42"), 0));
console.log(a(__parse_f64("-2.5"), 0));
console.log(a(__parse_f64("nope"), -(1)));
console.log(g(__parse_f64("3.14")));
console.log(g(__parse_f64("abc")));
