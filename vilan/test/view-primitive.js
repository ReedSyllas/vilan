function scale(target, factor) {
	target[0][target[1]] = target[0][target[1]] * factor;
}
let a = [ 10 ];
const c = [ a, 0 ];
c[0][c[1]] = 40;
console.log(a[0]);
let n = [ 5 ];
scale([ n, 0 ], 4);
console.log(n[0]);
