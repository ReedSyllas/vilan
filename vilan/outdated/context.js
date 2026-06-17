function c/*level_1*/(d) {
	console.log(d);
	e/*level_2*/(d);
}
function e/*level_2*/(f) {
	console.log(f);
	g/*level_3*/(f);
}
function g/*level_3*/(h) {
	console.log(h);
}
const a/*count_context*/ = null;
((b) => {
	return c/*level_1*/(b);
})(0);
