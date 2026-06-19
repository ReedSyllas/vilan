function default2() {
	return "";
}
function default3() {
	return false;
}
function default4() {
	return 0;
}
function eq(self, other) {
	return self[0] === other[0] && self[1] === other[1] && self[2] === other[2];
}
function default5() {
	return [ default4(), default2(), default3() ];
}
const d = default5();
console.log(d[0]);
console.log(d[1]);
console.log(d[2]);
const d2 = default5();
console.log(eq(d, d2));
