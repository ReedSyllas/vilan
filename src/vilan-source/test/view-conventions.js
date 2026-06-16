function b/*bump*/(c) {
	c[0] = c[0] + 1;
}
function d/*peek*/(e) {
	return e[0];
}
let a/*c*/ = [ 10 ];
b/*bump*/(a/*c*/);
console.log(a/*c*/[0]);
console.log(d/*peek*/(a/*c*/));
