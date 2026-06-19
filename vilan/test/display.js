function to_string4(self4) {
	return self4;
}
function to_string(self) {
	return "" + self;
}
function to_string2(self2) {
	return "" + self2;
}
function to_string3(self3) {
	return "" + self3;
}
function to_string5(self5) {
	return "(" + self5[0] + ", " + self5[1] + ")";
}
const n = 42;
console.log(to_string(n));
const x = 3.5;
console.log(to_string2(x));
console.log(to_string3(true));
console.log(to_string4("hi"));
const p = [ 1, 2 ];
console.log(to_string5(p));
