function debug(self) {
	return "Point { x = " + JSON.stringify(self[0]) + ", " + "y = " + JSON.stringify(self[1]) + " }";
}
function debug2(self2) {
	return "Tagged { label = " + JSON.stringify(self2[0]) + ", " + "at = " + debug(self2[1]) + ", " + "on = " + JSON.stringify(self2[2]) + " }";
}
function debug3(self3) {
	return "Empty";
}
console.log(debug([ 1, 2 ]));
const t = [ "hi", [ 3, 4 ], true ];
console.log(debug2(t));
console.log(debug3([  ]));
