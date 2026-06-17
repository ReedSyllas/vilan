function b/*increment*/(c) {
	c[0] = c[0] + 1;
}
function d/*bump*/(e, f) {
	e[0] = e[0] + f;
}
let a/*c*/ = [ 10 ];
a/*c*/[0] = 5;
console.log(a/*c*/[0]);
b/*increment*/(a/*c*/);
console.log(a/*c*/[0]);
d/*bump*/(a/*c*/, 100);
console.log(a/*c*/[0]);
let g/*x*/ = 1;
g/*x*/ = 2;
g/*x*/ = g/*x*/ + 3;
console.log(g/*x*/);
