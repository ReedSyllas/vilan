function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __list_get(list, index) {
	return index >= 0 && index < list.length ? [ 0, __clone(list[index]) ] : [ 1 ];
}
function __list_pop(list) {
	return list.length === 0 ? [ 1 ] : [ 0, list.pop() ];
}
function __shared_new(value) {
	return { v: value };
}
function fresh_id() {
	const id = next_subscriber_id.v;
	next_subscriber_id.v = id + 1;
	return id;
}
function enqueue(turn, subscribers) {
	for (const subscriber of subscribers) {
		let seen = false;
		for (const queued of turn[0].v) {
			if (queued[0] === subscriber[0]) {
				seen = true;
			}
		}
		if (!(seen)) {
			turn[0].v.push(subscriber);
		}
	}
	if (turn[2].v && !(turn[3].v) && !(turn[1].v)) {
		turn[3].v = true;
		queueMicrotask(() => {
			turn[3].v = false;
			drain(turn);
			return;
		});
	}
}
function drain(turn) {
	if (!(turn[1].v)) {
		turn[1].v = true;
		draining_turns.v.push(turn);
		let budget = 100000;
		while (!($p(turn[0].v)) && budget > 0) {
			const wave = turn[0].v;
			turn[0].v = [  ];
			for (const subscriber of wave) {
				subscriber[1]();
				budget = budget - 1;
			}
		}
		__list_pop(draining_turns.v);
		turn[1].v = false;
	}
}
function dispose(self, $i) {
	let kept = [  ];
	for (const subscriber of self[0].v) {
		if (subscriber[0] !== self[1]) {
			kept.push(subscriber);
		}
	}
	self[0].v = kept;
	const $j = $i;
	let $k = null;
	if ($j[0] === 0) {
		const turn = $j[1];
		let kept_pending = [  ];
		for (const subscriber2 of turn[0].v) {
			if (subscriber2[0] !== self[1]) {
				kept_pending.push(subscriber2);
			}
		}
		turn[0].v = kept_pending;
		$k = undefined;
	} else {
		$k = undefined;
	}
	return $k;
}
function $a(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $b(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $e(self) {
	return self[0].v;
}
function $f(self) {
	return self[0].v;
}
function $p(self) {
	return self.length === 0;
}
function $q(self) {
	return __list_get(self, self.length - 1);
}
function $l(self, value, $m) {
	self[0].v = value;
	const $n = $m;
	let $o = null;
	if ($n[0] === 0) {
		const turn = $n[1];
		$o = enqueue(turn, self[1].v);
	} else {
		const $r = $q(draining_turns.v);
		let $s = null;
		if ($r[0] === 0) {
			const draining = $r[1];
			$s = enqueue(draining, self[1].v);
		} else {
			for (const subscriber of self[1].v) {
				subscriber[1]();
			}
			$s = undefined;
		}
		$o = $s;
	}
	return $o;
}
function $t(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($f(self));
		return;
	} ]);
	observer($f(self));
	return [ self[1], id ];
}
function $u(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($e(self));
		return;
	} ]);
	observer($e(self));
	return [ self[1], id ];
}
function $c(self, $d) {
	const derived = $a($f($e(self)));
	const inner_subscription = __shared_new([ 1 ]);
	$u(self, (inner) => {
		const $g = inner_subscription.v;
		let $h = null;
		if ($g[0] === 1) {
			$h = $g;
		} else {
			$h = [ 0, dispose($g[1], $d) ];
		}
		$h;
		inner_subscription.v = [ 0, $t(inner, (value) => {
			$l(derived, value, $d);
			return;
		}) ];
		return;
	});
	return derived;
}
function $v(self, value, $m) {
	self[0].v = value;
	const $w = $m;
	let $x = null;
	if ($w[0] === 0) {
		const turn = $w[1];
		$x = enqueue(turn, self[1].v);
	} else {
		const $y = $q(draining_turns.v);
		let $z = null;
		if ($y[0] === 0) {
			const draining = $y[1];
			$z = enqueue(draining, self[1].v);
		} else {
			for (const subscriber of self[1].v) {
				subscriber[1]();
			}
			$z = undefined;
		}
		$x = $z;
	}
	return $x;
}
function $A(self, transform, $B) {
	const derived = $a(transform($f(self)));
	self[1].v.push([ fresh_id(), () => {
		$l(derived, transform($f(self)), $B);
		return;
	} ]);
	return derived;
}
const next_subscriber_id = __shared_new(0);
const turn_scope = null;
const draining_turns = __shared_new([  ]);
const owner_scope = null;
const first = $a(1);
const second = $a(10);
const outer = $b(first);
const joined = $c(outer, [ 1 ]);
console.log($f(joined));
$l(first, 2, [ 1 ]);
console.log($f(joined));
$v(outer, second, [ 1 ]);
console.log($f(joined));
$l(first, 99, [ 1 ]);
console.log($f(joined));
$l(second, 11, [ 1 ]);
console.log($f(joined));
const doubled = $A(joined, (value) => {
	return value * 2;
}, [ 1 ]);
$l(second, 21, [ 1 ]);
console.log($f(doubled));
