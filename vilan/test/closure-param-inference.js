function fold(self5, init, fn4) {
	let accumulator = init;
	for (const item2 of self5) {
		accumulator = fn4(accumulator, item2);
	}
	return accumulator;
}
function $a(self, fn) {
	const $b = self;
	let $c = null;
	if ($b[0] === 0) {
		const x = $b[1];
		$c = [ 0, fn(x) ];
	} else {
		$c = [ 1 ];
	}
	return $c;
}
function $d(self2, fallback) {
	const $e = self2;
	let $f = null;
	if ($e[0] === 0) {
		const x2 = $e[1];
		$f = x2;
	} else {
		$f = fallback;
	}
	return $f;
}
function $g(self3, fn2) {
	const $h = self3;
	let $i = null;
	if ($h[0] === 0) {
		const x3 = $h[1];
		$i = fn2(x3);
	} else {
		$i = false;
	}
	return $i;
}
function $j(self4, fn3) {
	let result = [  ];
	for (const item of self4) {
		result.push(fn3(item));
	}
	return result;
}
function $k(self6, predicate) {
	let result2 = [  ];
	for (const item3 of self6) {
		if (predicate(item3)) {
			result2.push(item3);
		}
	}
	return result2;
}
const p = [ 0, [ 3, 4 ] ];
console.log($d($a(p, (q) => {
	return q[0] + q[1];
}), 0));
console.log($g(p, (q2) => {
	return q2[0] === 3;
}));
let pts = [  ];
pts.push([ 1, 10 ]);
pts.push([ 2, 20 ]);
console.log(fold($j(pts, (pt) => {
	return pt[0];
}), 0, (a, b) => {
	return a + b;
}));
console.log($k(pts, (pt2) => {
	return pt2[1] > 15;
}).length);
