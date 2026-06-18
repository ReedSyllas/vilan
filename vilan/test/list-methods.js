function b/*is_empty*/(c) {
	return c.length === 0;
}
function e/*map*/(f, g) {
	let h/*result*/ = [  ];
	for (const i/*item*/ of f) {
		h/*result*/.push(g(i/*item*/));
	}
	return h/*result*/;
}
function s/*filter*/(t, u) {
	let v/*result*/ = [  ];
	for (const w/*item*/ of t) {
		if (u(w/*item*/)) {
			v/*result*/.push(w/*item*/);
		}
	}
	return v/*result*/;
}
function l/*fold*/(m, n, o) {
	let p/*accumulator*/ = n;
	for (const q/*item*/ of m) {
		p/*accumulator*/ = o(p/*accumulator*/, q/*item*/);
	}
	return p/*accumulator*/;
}
function z/*for_each*/(A, B) {
	for (const C/*item*/ of A) {
		B(C/*item*/);
	}
}
let a/*xs*/ = [  ];
a/*xs*/.push(1);
a/*xs*/.push(2);
a/*xs*/.push(3);
a/*xs*/.push(4);
console.log(a/*xs*/.length);
console.log(b/*is_empty*/(a/*xs*/));
console.log(l/*fold*/(e/*map*/(a/*xs*/, (d) => {
	return d * 10;
}), 0, (j, k) => {
	return j + k;
}));
console.log(s/*filter*/(a/*xs*/, (r) => {
	return r > 2;
}).length);
console.log(s/*filter*/(a/*xs*/, (x) => {
	return x > 5;
}).length);
z/*for_each*/(a/*xs*/, (y) => {
	return console.log(y);
});
