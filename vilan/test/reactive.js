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
		while (!($i(turn[0].v)) && budget > 0) {
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
function dispose(self, $p) {
	let kept = [  ];
	for (const subscriber of self[0].v) {
		if (subscriber[0] !== self[1]) {
			kept.push(subscriber);
		}
	}
	self[0].v = kept;
	const $q = $p;
	let $r = null;
	if ($q[0] === 0) {
		const turn = $q[1];
		let kept_pending = [  ];
		for (const subscriber2 of turn[0].v) {
			if (subscriber2[0] !== self[1]) {
				kept_pending.push(subscriber2);
			}
		}
		turn[0].v = kept_pending;
		$r = undefined;
	} else {
		$r = undefined;
	}
	return $r;
}
function new2() {
	return [ __shared_new([  ]) ];
}
function $a(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $d(self) {
	return self[0].v;
}
function $i(self) {
	return self.length === 0;
}
function $j(self) {
	return __list_get(self, self.length - 1);
}
function $e(self, value, $f) {
	self[0].v = value;
	const $g = $f;
	let $h = null;
	if ($g[0] === 0) {
		const turn = $g[1];
		$h = enqueue(turn, self[1].v);
	} else {
		const $k = $j(draining_turns.v);
		let $l = null;
		if ($k[0] === 0) {
			const draining = $k[1];
			$l = enqueue(draining, self[1].v);
		} else {
			for (const subscriber of self[1].v) {
				subscriber[1]();
			}
			$l = undefined;
		}
		$h = $l;
	}
	return $h;
}
function $b(self, transform, $c) {
	const derived = $a(transform($d(self)));
	self[1].v.push([ fresh_id(), () => {
		$e(derived, transform($d(self)), $c);
		return;
	} ]);
	return derived;
}
function $m(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($d(self));
		return;
	} ]);
	observer($d(self));
	return [ self[1], id ];
}
function $n(self, item, $o) {
	self[0].v.push(() => {
		dispose(item, $o);
		return;
	});
	return item;
}
function $s(self, transform, $t) {
	$e(self, transform($d(self)), $t);
}
const next_subscriber_id = __shared_new(0);
const turn_scope = null;
const draining_turns = __shared_new([  ]);
const owner_scope = null;
const owner = new2();
const count = $a(0);
const doubled = $b(count, (n) => {
	return n * 2;
}, [ 1 ]);
$n(owner, $m(doubled, (n) => {
	return console.log(n);
}), [ 1 ]);
$e(count, 1, [ 1 ]);
$s(count, (n) => {
	return n + 4;
}, [ 1 ]);
console.log($d(doubled));
$n(owner, $m(count, (n) => {
	return console.log(n);
}), [ 1 ]);
$e(count, 20, [ 1 ]);
