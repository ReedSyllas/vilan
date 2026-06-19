function b/*slot*/(c) {
	return [ c, 0 ];
}
let a/*w*/ = [ 1 ];
const d = b/*slot*/(a/*w*/);
d[0][d[1]] = 10;
console.log(a/*w*/[0]);
const e/*v*/ = b/*slot*/(a/*w*/);
console.log(e/*v*/[0][e/*v*/[1]]);
e/*v*/[0][e/*v*/[1]] = 25;
console.log(a/*w*/[0]);
