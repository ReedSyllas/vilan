function __args() {
	return process.argv.slice(2);
}
function __env(key) {
	const value = process.env[key];
	return value === undefined ? [ 1 ] : [ 0, value ];
}
function e(f, g) {
	const h = f;
	let i = null;
	if (h[0] === 0) {
		const j/*x*/ = h[1];
		i = j/*x*/;
	} else {
		i = g;
	}
	return i;
}
const a/*arguments*/ = __args();
console.log(a/*arguments*/.length);
const b = __env("VILAN_TEST_VAR");
let c = null;
if (b[0] === 0) {
	const d/*value*/ = b[1];
	c = console.log(d/*value*/);
} else {
	c = console.log("unset");
}
c;
console.log(e(__env("DEFINITELY_NOT_SET_XYZ"), "unset"));
