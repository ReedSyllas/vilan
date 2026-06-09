function d/*new*/(e) {
	return [ e ];
}
function c/*default*/() {
	return d/*new*/(0);
}
function b() {
	return c/*default*/();
}
const a/*my_id*/ = b();
console.log(a/*my_id*/);
