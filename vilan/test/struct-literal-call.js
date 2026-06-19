function sum(self) {
	return self[0] + self[1];
}
function shifted(self) {
	return [ self[0] + 1, self[1] + 1 ];
}
console.log(sum([ 3, 4 ]));
console.log([ 3, 4 ][0]);
console.log(sum(shifted([ 10, 20 ])));
