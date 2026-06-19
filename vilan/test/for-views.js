let xs = [  ];
xs.push(1);
xs.push(2);
xs.push(3);
const $a = xs;
for (const $b of $a.keys()) {
	const e = [ $a, $b ];
	e[0][e[1]] = e[0][e[1]] * 10;
}
console.log(xs[0]);
console.log(xs[2]);
let ps = [  ];
ps.push([ 1 ]);
ps.push([ 2 ]);
const $c = ps;
for (const $d of $c.keys()) {
	const p = $c[$d];
	p[0] = p[0] + 100;
}
console.log(ps[0][0]);
console.log(ps[1][0]);
let sum = 0;
const $e = xs;
for (const $f of $e.keys()) {
	const e2 = [ $e, $f ];
	sum = sum + e2[0][e2[1]];
}
console.log(sum);
