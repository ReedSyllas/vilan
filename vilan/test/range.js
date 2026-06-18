function b/*new*/(c, d) {
	return [ c, d ];
}
function e/*next*/(f) {
	let h = null;
	if (f[0] < f[1]) {
		const g/*value*/ = f[0];
		f[0] = f[0] + 1;
		h = [ 0, g/*value*/ ];
	} else {
		h = [ 1 ];
	}
	return h;
}
let a/*sum*/ = 0;
const i = b/*new*/(1, 10);
while (true) {
	const j = e/*next*/(i);
	if (j[0] !== 0) {
		break;
	}
	const k/*i*/ = j[1];
	a/*sum*/ = a/*sum*/ + k/*i*/;
}
console.log(a/*sum*/);
let l/*count*/ = 0;
const m = b/*new*/(0, 5);
while (true) {
	const n = e/*next*/(m);
	if (n[0] !== 0) {
		break;
	}
	const o/*j*/ = n[1];
	l/*count*/ = l/*count*/ + 1;
}
console.log(l/*count*/);
let p/*empty*/ = 0;
const q = b/*new*/(3, 3);
while (true) {
	const r = e/*next*/(q);
	if (r[0] !== 0) {
		break;
	}
	const s/*k*/ = r[1];
	p/*empty*/ = p/*empty*/ + 1;
}
console.log(p/*empty*/);
let t/*squares*/ = [  ];
const u = b/*new*/(1, 5);
while (true) {
	const v = e/*next*/(u);
	if (v[0] !== 0) {
		break;
	}
	const w/*n*/ = v[1];
	t/*squares*/.push(Math.pow(w/*n*/, 2));
}
console.log(t/*squares*/.length);
