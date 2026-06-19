function d/*bump*/(e) {
	e[0][e[1]] = e[0][e[1]] + 1;
}
let a/*c*/ = [ 1 ];
const b/*v*/ = [ a/*c*/, 0 ];
b/*v*/[0][b/*v*/[1]] = 10;
console.log(a/*c*/[0]);
let c/*p*/ = [ 5, 7 ];
d/*bump*/([ c/*p*/, 1 ]);
console.log(c/*p*/[1]);
const f/*r*/ = [ c/*p*/, 0 ];
console.log(f/*r*/[0][f/*r*/[1]]);
f/*r*/[0][f/*r*/[1]] = f/*r*/[0][f/*r*/[1]] * 3;
console.log(c/*p*/[0]);
let g/*q*/ = [ 1, 2 ];
const h/*w*/ = g/*q*/;
Object.assign(h/*w*/, [ 7, 8 ]);
console.log(g/*q*/[0]);
console.log(g/*q*/[1]);
