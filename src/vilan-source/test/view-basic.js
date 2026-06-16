let a/*c*/ = [ 10 ];
const b/*v*/ = a/*c*/;
b/*v*/[0] = 99;
console.log(a/*c*/[0]);
let c/*e*/ = [ 10 ];
let d/*d*/ = structuredClone(c/*e*/);
d/*d*/[0] = 1;
console.log(c/*e*/[0]);
