function d/*new*/(e) {
	return [ e ];
}
function c/*default*/() {
	return d/*new*/(0);
}
function b() {
	return c/*default*/();
}
const a/*some_id*/ = b();
console.log(a/*some_id*/);
