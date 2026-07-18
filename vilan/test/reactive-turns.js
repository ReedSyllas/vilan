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
class __Task {
	constructor(run, origin) {
		this.origin = origin;
		this.observed = false;
		this.promise = run();
		this.promise.then(null, (error) => {
			if (!this.observed) {
				globalThis.setTimeout(() => {
					if (!this.observed) console.error("unhandled task error (spawned in " + this.origin + "): " + String(error));
				}, 0);
			}
		});
	}
	then(onFulfilled, onRejected) {
		this.observed = true;
		return this.promise.then(onFulfilled, onRejected);
	}
}
function __task(run, origin) {
	return new __Task(run, origin);
}
function fresh_id() {
	const id = next_subscriber_id.v;
	next_subscriber_id.v = id + 1;
	return id;
}
function new2() {
	return [ __shared_new([  ]), __shared_new(false), __shared_new(false), __shared_new(false) ];
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
function flush($n) {
	const $o = $n;
	let $p = null;
	if ($o[0] === 0) {
		const turn = $o[1];
		$p = drain(turn);
	} else {
		$p = undefined;
	}
	return $p;
}
async function tick() {

}
function $a(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $c(self) {
	return self[0].v;
}
function $b(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($c(self));
		return;
	} ]);
	observer($c(self));
	return [ self[1], id ];
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
function $s(policy, body) {
	const fresh = new2();
	const result = body(fresh);
	drain(fresh);
	fresh[2].v = true;
	return result;
}
function $u(body, $v) {
	const $w = $v;
	let $x = null;
	if ($w[0] === 0) {
		const current = $w[1];
		$x = body(current);
	} else {
		const fresh = new2();
		const result = body(fresh);
		drain(fresh);
		fresh[2].v = true;
		$x = result;
	}
	return $x;
}
const next_subscriber_id = __shared_new(0);
const draining_turns = __shared_new([  ]);
const a = $a(0);
const b = $a(0);
$b(a, (value) => {
	return console.log("a -> " + value);
});
$b(b, (value) => {
	return console.log("b -> " + value);
});
const turn_a = new2();
const turn_b = new2();
(($d) => {
	$e(a, 1, [ 0, $d ]);
	return;
})(turn_a);
(($m) => {
	$e(b, 1, [ 0, $m ]);
	flush([ 0, $m ]);
	return;
})(turn_b);
console.log("mid");
(($q) => {
	return flush([ 0, $q ]);
})(turn_a);
$s([ 0 ], ($r) => {
	$e(a, 2, [ 0, $r ]);
	$e(b, 2, [ 0, $r ]);
	console.log("inside");
	return;
});
$u(($t) => {
	$e(a, 3, [ 0, $t ]);
	console.log("batched");
	return;
}, [ 1 ]);
$s([ 0 ], ($y) => {
	$u(($z) => {
		$e(a, 4, [ 0, $z ]);
		return;
	}, [ 0, $y ]);
	console.log("joined");
	return;
});
const turn_c = new2();
(($A) => {
	__task(async () => {
		await (await (tick()));
		$e(a, 5, [ 0, $A ]);
		flush([ 0, $A ]);
		return;
	}, "main");
	return;
})(turn_c);
console.log("end-sync");
