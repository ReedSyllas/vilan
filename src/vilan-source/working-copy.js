function c /* new_rect */() {
	return [ a /* RECTANGLE */, "Rectangle", 10, 20, 70, 30 ];
}
function d /* f */(e) {
	console.log("hello from shape method `f`");
	console.log(e[1]);
	return;
}
const a /* RECTANGLE */ = 0;
const b /* shape */ = c /* new_rect */();
console.log(d /* f */(b /* shape */));
