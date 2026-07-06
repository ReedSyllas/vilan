function to_string(self) {
	return "" + self;
}
function to_string2(self) {
	return "x=" + $b(self[0]) + ", " + "y=" + $b(self[1]);
}
function to_string3(self) {
	return "width=" + $b(self[0]) + ", " + "height=" + $b(self[1]);
}
function $b(value) {
	return to_string(value);
}
function $a(value) {
	return to_string2(value);
}
function $c(value) {
	return to_string3(value);
}
console.log($a([ 1, 2 ]));
console.log($c([ 3, 4 ]));
