function e/*sum*/(f) {
	return [ 5, g/*bar*/(f) ];
}
function b/*new*/(c, d) {
	return [ c, d ];
}
function g/*bar*/(h) {
	return h[1](h[0]);
}
console.log(e/*sum*/(b/*new*/(3, (a) => {
	return a * a;
})));
