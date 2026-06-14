function f/*compare*/(g, h) {
	let i = null;
	if (g[0] < h[0]) {
		i = -1;
	} else if (g[0] > h[0]) {
		i = 1;
	} else {
		i = 0;
	}
	return i;
}
function c(d, e) {
	let j = null;
	if (f/*compare*/(d, e) <= 0) {
		j = d;
	} else {
		j = e;
	}
	return j;
}
function k(l, m) {
	let n = null;
	if (f/*compare*/(l, m) >= 0) {
		n = l;
	} else {
		n = m;
	}
	return n;
}
function r(s, t, u) {
	return k(c(s, u), t);
}
const a/*low*/ = [ 3 ];
const b/*high*/ = [ 7 ];
console.log(c(a/*low*/, b/*high*/));
console.log(k(a/*low*/, b/*high*/));
const o/*below*/ = [ 1 ];
const p/*above*/ = [ 9 ];
const q/*within*/ = [ 5 ];
console.log(r(o/*below*/, a/*low*/, b/*high*/));
console.log(r(p/*above*/, a/*low*/, b/*high*/));
console.log(r(q/*within*/, a/*low*/, b/*high*/));
