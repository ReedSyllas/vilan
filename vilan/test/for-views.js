let a/*xs*/ = [  ];
a/*xs*/.push(1);
a/*xs*/.push(2);
a/*xs*/.push(3);
const b = a/*xs*/;
for (const c of b.keys()) {
	const d/*e*/ = [ b, c ];
	d/*e*/[0][d/*e*/[1]] = d/*e*/[0][d/*e*/[1]] * 10;
}
console.log(a/*xs*/[0]);
console.log(a/*xs*/[2]);
let e/*ps*/ = [  ];
e/*ps*/.push([ 1 ]);
e/*ps*/.push([ 2 ]);
const f = e/*ps*/;
for (const g of f.keys()) {
	const h/*p*/ = f[g];
	h/*p*/[0] = h/*p*/[0] + 100;
}
console.log(e/*ps*/[0][0]);
console.log(e/*ps*/[1][0]);
let i/*sum*/ = 0;
const j = a/*xs*/;
for (const k of j.keys()) {
	const l/*e*/ = [ j, k ];
	i/*sum*/ = i/*sum*/ + l/*e*/[0][l/*e*/[1]];
}
console.log(i/*sum*/);
