function sum(self) {
	return self[0] + self[1];
}
function shifted(self2) {
	return [ self2[0] + 1, self2[1] + 1 ];
}
console.log(sum([ 3, 4 ]));
console.log([ 3, 4 ][0]);
console.log(sum(shifted([ 10, 20 ])));
