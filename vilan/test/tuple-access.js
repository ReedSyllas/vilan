function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
const pair = [ 7, "vilan" ];
console.log(pair[0]);
console.log(pair[1].length);
const nested = [ ...[ 1, 2 ], 3 ];
console.log(nested[1]);
const inner = __clone(nested.slice(0, 2));
console.log(inner[0] + inner[1] + nested[2]);
let counter = [ 0, 100 ];
counter[0] = counter[0] + 1;
counter[1] = counter[1] - 1;
console.log(counter[0] + counter[1]);
let deep = [ ...[ 1, 2 ], 3 ];
const $a = [ 40, 2 ];
deep[0] = $a[0];
deep[1] = $a[1];
console.log(deep[0] + deep[1] + deep[2]);
deep[1] = 7;
console.log(deep[1]);
