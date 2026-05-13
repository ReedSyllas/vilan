function c/*new_rect*/() {
	return [ a/*RECTANGLE*/, "Rectangle", 10, 20, 70, 30 ];
}
function e/*f*/(f, g) {
	console.log("hello from shape method `f` with message:");
	console.log(g("hi"));
	console.log(f[1]);
	return;
}
const a/*RECTANGLE*/ = 0;
const b/*shape*/ = c/*new_rect*/();
e/*f*/(b/*shape*/, (d) => {
	return d;
});
