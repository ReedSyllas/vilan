function eq(self, other) {
	return self[0] === other[0] && self[1] === other[1];
}
function eq2(self2, other2) {
	return true;
}
const a = [ 1, 2 ];
const b = [ 1, 2 ];
const c = [ 9, 2 ];
console.log(eq(a, b));
console.log(eq(a, c));
console.log(!(eq(a, c)));
const u1 = [  ];
const u2 = [  ];
console.log(eq2(u1, u2));
