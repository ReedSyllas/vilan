function i/*to_string*/(j) {
	return j;
}
function b/*to_string*/(c) {
	return "" + c;
}
function e/*to_string*/(f) {
	return "" + f;
}
function g/*to_string*/(h) {
	return "" + h;
}
function l/*to_string*/(m) {
	return "(" + m[0] + ", " + m[1] + ")";
}
const a/*n*/ = 42;
console.log(b/*to_string*/(a/*n*/));
const d/*x*/ = 3.5;
console.log(e/*to_string*/(d/*x*/));
console.log(g/*to_string*/(true));
console.log(i/*to_string*/("hi"));
const k/*p*/ = [ 1, 2 ];
console.log(l/*to_string*/(k/*p*/));
