function default4() {
	return "";
}
function default5() {
	return false;
}
function default3() {
	return 0;
}
function eq(self, other) {
	return self[0] === other[0] && self[1] === other[1] && self[2] === other[2];
}
function default2() {
	return [ default3(), default4(), default5() ];
}
const d = default2();
console.log(d[0]);
console.log(d[1]);
console.log(d[2]);
const d2 = default2();
console.log(eq(d, d2));
