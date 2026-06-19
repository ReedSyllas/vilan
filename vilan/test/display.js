function to_string(self) {
	return self;
}
function to_string2(self) {
	return "" + self;
}
function to_string3(self) {
	return "" + self;
}
function to_string4(self) {
	return "" + self;
}
function to_string5(self) {
	return "(" + self[0] + ", " + self[1] + ")";
}
const n = 42;
console.log(to_string2(n));
const x = 3.5;
console.log(to_string3(x));
console.log(to_string4(true));
console.log(to_string("hi"));
const p = [ 1, 2 ];
console.log(to_string5(p));
