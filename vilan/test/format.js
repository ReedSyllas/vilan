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
	return "Id { n = " + to_string2(self[0]) + " }";
}
function to_string5(self) {
	return "Point { x = " + to_string2(self[0]) + ", y = " + to_string2(self[1]) + " }";
}
function $a(value) {
	return to_string4(value);
}
function $b(value) {
	return to_string5(value);
}
function $c(value) {
	return to_string2(value);
}
function $d(value) {
	return to_string(value);
}
function $e(value) {
	return to_string3(value);
}
const id = [ 0 ];
console.log(id);
console.log($a(id));
console.log($b([ 1, 2 ]));
console.log($c(42));
console.log($d("hi"));
console.log($e(true));
