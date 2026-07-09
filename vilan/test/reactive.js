function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __list_get(list, index) {
	return index >= 0 && index < list.length ? [ 0, __clone(list[index]) ] : [ 1 ];
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
}
function dispose(self, $o) {
	let kept = [  ];
	for (const subscriber of self[0].v) {
		if (subscriber[0] !== self[1]) {
			kept.push(subscriber);
		}
	}
	self[0].v = kept;
	const $p = $o;
	let $q = null;
	if ($p[0] === 0) {
		const turn = $p[1];
		let kept_pending = [  ];
		for (const subscriber2 of turn[0].v) {
			if (subscriber2[0] !== self[1]) {
				kept_pending.push(subscriber2);
			}
		}
		turn[0].v = kept_pending;
		$q = undefined;
	} else {
		$q = undefined;
	}
	return $q;
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
		const $j = $i(draining_turns.v);
		let $k = null;
		if ($j[0] === 0) {
			const draining = $j[1];
			$k = enqueue(draining, self[1].v);
		} else {
			for (const subscriber of self[1].v) {
				subscriber[1]();
			}
			$k = undefined;
		}
		$h = $k;
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
function $l(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($d(self));
		return;
	} ]);
	observer($d(self));
	return [ self[1], id ];
}
function $m(self, item, $n) {
	self[0].v.push(() => {
		dispose(item, $n);
		return;
	});
	return item;
}
function $r(self, transform, $s) {
	$e(self, transform($d(self)), $s);
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
$m(owner, $l(doubled, (n) => {
	return console.log(n);
}), [ 1 ]);
$e(count, 1, [ 1 ]);
$r(count, (n) => {
	return n + 4;
}, [ 1 ]);
console.log($d(doubled));
$m(owner, $l(count, (n) => {
	return console.log(n);
}), [ 1 ]);
$e(count, 20, [ 1 ]);
