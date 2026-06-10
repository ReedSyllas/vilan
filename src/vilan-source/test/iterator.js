function e/*next*/(f) {
	return f[0]();
}
function c/*from_fn*/(d) {
	return [ d ];
}
let a/*i*/ = 0;
const b/*naturals*/ = c/*from_fn*/(() => {
	a/*i*/ = a/*i*/ + 1;
	return a/*i*/;
});
console.log(e/*next*/(b/*naturals*/));
console.log(e/*next*/(b/*naturals*/));
console.log(e/*next*/(b/*naturals*/));
