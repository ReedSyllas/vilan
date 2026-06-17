function b/*increment*/(c) {
	c[0] = c[0] + 1;
}
function d/*bump*/(e) {
	e[0] = e[0] + 10;
}
let a/*c*/ = [ 10 ];
b/*increment*/(a/*c*/);
d/*bump*/(a/*c*/);
console.log(a/*c*/[0]);
