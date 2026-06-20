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
function $d(self, fallback) {
	const $e = self;
	let $f = null;
	if ($e[0] === 0) {
		const x = $e[1];
		$f = x;
	} else {
		$f = fallback;
	}
	return $f;
}
function $g(self, fn) {
	const $h = self;
	let $i = null;
	if ($h[0] === 0) {
		const x = $h[1];
		$i = fn(x);
	} else {
		$i = false;
	}
	return $i;
}
function $j(self, fn) {
	let result = [  ];
	for (const item of self) {
		result.push(fn(item));
	}
	return result;
}
function $k(self, init, fn) {
	let accumulator = init;
	for (const item of self) {
		accumulator = fn(accumulator, item);
	}
	return accumulator;
}
function $l(self, predicate) {
	let result = [  ];
	for (const item of self) {
		if (predicate(item)) {
			result.push(item);
		}
	}
	return result;
}
const p = [ 0, [ 3, 4 ] ];
console.log($d($a(p, (q) => {
	return q[0] + q[1];
}), 0));
console.log($g(p, (q) => {
	return q[0] === 3;
}));
let pts = [  ];
pts.push([ 1, 10 ]);
pts.push([ 2, 20 ]);
console.log($k($j(pts, (pt) => {
	return pt[0];
}), 0, (a, b) => {
	return a + b;
}));
console.log($l(pts, (pt) => {
	return pt[1] > 15;
}).length);
