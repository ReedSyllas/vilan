function c/*from_fn*/(d) {
	return [ d ];
}
function e(f) {
	return f[0]();
}
let a/*i*/ = 0;
const b/*naturals*/ = c/*from_fn*/(() => {
	a/*i*/ = a/*i*/ + 1;
	return a/*i*/;
});
console.log(e(b/*naturals*/));
console.log(e(b/*naturals*/));
console.log(e(b/*naturals*/));
