function d/*default*/() {
	return "";
}
function e/*default*/() {
	return false;
}
function c/*default*/() {
	return 0;
}
function g/*eq*/(h, i) {
	return h[0] === i[0] && h[1] === i[1] && h[2] === i[2];
}
function b/*default*/() {
	return [ c/*default*/(), d/*default*/(), e/*default*/() ];
}
const a/*d*/ = b/*default*/();
console.log(a/*d*/[0]);
console.log(a/*d*/[1]);
console.log(a/*d*/[2]);
const f/*d2*/ = b/*default*/();
console.log(g/*eq*/(a/*d*/, f/*d2*/));
