function a/*debug*/(b) {
	return "Point { x = " + JSON.stringify(b[0]) + ", " + "y = " + JSON.stringify(b[1]) + " }";
}
function d/*debug*/(e) {
	return "Tagged { label = " + JSON.stringify(e[0]) + ", " + "at = " + a/*debug*/(e[1]) + ", " + "on = " + JSON.stringify(e[2]) + " }";
}
function f/*debug*/(g) {
	return "Empty";
}
console.log(a/*debug*/([ 1, 2 ]));
const c/*t*/ = [ "hi", [ 3, 4 ], true ];
console.log(d/*debug*/(c/*t*/));
console.log(f/*debug*/([  ]));
