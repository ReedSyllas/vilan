function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function c/*bump*/(d) {
	d[0][d[1]] = d[0][d[1]] + 100;
}
let a/*xs*/ = [  ];
a/*xs*/.push(10);
a/*xs*/.push(20);
console.log(a/*xs*/[0] + a/*xs*/[1]);
a/*xs*/[1] = 99;
console.log(a/*xs*/[1]);
const b/*i*/ = 0;
c/*bump*/([ a/*xs*/, b/*i*/ + 0 ]);
console.log(a/*xs*/[0]);
let e/*ps*/ = [  ];
e/*ps*/.push([ 1, 2 ]);
let f/*copy*/ = __clone(e/*ps*/[0]);
f/*copy*/[0] = 7;
console.log(e/*ps*/[0][0]);
const g/*view*/ = e/*ps*/[0];
g/*view*/[1] = 50;
console.log(e/*ps*/[0][1]);
console.log(a/*xs*/[a/*xs*/.length - 1]);
