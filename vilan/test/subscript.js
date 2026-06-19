function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function bump(slot) {
	slot[0][slot[1]] = slot[0][slot[1]] + 100;
}
let xs = [  ];
xs.push(10);
xs.push(20);
console.log(xs[0] + xs[1]);
xs[1] = 99;
console.log(xs[1]);
const i = 0;
bump([ xs, i + 0 ]);
console.log(xs[0]);
let ps = [  ];
ps.push([ 1, 2 ]);
let copy = __clone(ps[0]);
copy[0] = 7;
console.log(ps[0][0]);
const view = ps[0];
view[1] = 50;
console.log(ps[0][1]);
console.log(xs[xs.length - 1]);
