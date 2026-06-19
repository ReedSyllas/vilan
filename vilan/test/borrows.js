function b/*slot*/(c) {
	return [ c, 0 ];
}
let a/*w*/ = [ 1 ];
b/*slot*/(a/*w*/)[0][b/*slot*/(a/*w*/)[1]] = 10;
console.log(a/*w*/[0]);
const d/*v*/ = b/*slot*/(a/*w*/);
console.log(d/*v*/[0][d/*v*/[1]]);
d/*v*/[0][d/*v*/[1]] = 25;
console.log(a/*w*/[0]);
