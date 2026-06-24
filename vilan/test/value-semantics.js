function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function default2() {
	return 0;
}
function $a(self) {
	let total = default2();
	let seeded = false;
	for (const item of self) {
		if (seeded) {
			total = total + item;
		} else {
			total = item;
			seeded = true;
		}
	}
	return total;
}
let a = [ 1, 2 ];
let b = __clone(a);
b[0] = 99;
console.log(a[0]);
console.log(b[0]);
let xs = [  ];
xs.push(1);
xs.push(2);
let ys = __clone(xs);
ys.push(99);
console.log($a(xs));
console.log($a(ys));
