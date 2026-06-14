function a/*sum*/(b) {
	return b[0] + b[1];
}
function c/*shifted*/(d) {
	return [ d[0] + 1, d[1] + 1 ];
}
console.log(a/*sum*/([ 3, 4 ]));
console.log([ 3, 4 ][0]);
console.log(a/*sum*/(c/*shifted*/([ 10, 20 ])));
