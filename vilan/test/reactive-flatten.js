function __shared_new(value) {
	return { v: value };
}
function fresh_id() {
	const id = next_subscriber_id.v;
	next_subscriber_id.v = id + 1;
	return id;
}
function enqueue(subscribers) {
	for (const subscriber of subscribers) {
		let seen = false;
		for (const queued of scheduler[0].v) {
			if (queued[0] === subscriber[0]) {
				seen = true;
			}
		}
		if (!(seen)) {
			scheduler[0].v.push(subscriber);
		}
	}
}
function dispose(self) {
	let kept = [  ];
	for (const subscriber of self[0].v) {
		if (subscriber[0] !== self[1]) {
			kept.push(subscriber);
		}
	}
	self[0].v = kept;
	let kept_pending = [  ];
	for (const subscriber2 of scheduler[0].v) {
		if (subscriber2[0] !== self[1]) {
			kept_pending.push(subscriber2);
		}
	}
	scheduler[0].v = kept_pending;
}
function $a(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $b(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $d(self) {
	return self[0].v;
}
function $e(self) {
	return self[0].v;
}
function $h(self, value) {
	self[0].v = value;
	let $i = null;
	if (scheduler[1].v === 0) {
		for (const subscriber of self[1].v) {
			subscriber[1]();
		}
		$i = undefined;
	} else {
		enqueue(self[1].v);
	}
	return $i;
}
function $j(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($e(self));
		return;
	} ]);
	observer($e(self));
	return [ self[1], id ];
}
function $k(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($d(self));
		return;
	} ]);
	observer($d(self));
	return [ self[1], id ];
}
function $c(self) {
	const derived = $a($e($d(self)));
	const inner_subscription = __shared_new([ 1 ]);
	$k(self, (inner) => {
		const $f = inner_subscription.v;
		let $g = null;
		if ($f[0] === 1) {
			$g = $f;
		} else {
			$g = [ 0, dispose($f[1]) ];
		}
		$g;
		inner_subscription.v = [ 0, $j(inner, (value) => {
			$h(derived, value);
			return;
		}) ];
		return;
	});
	return derived;
}
function $l(self, value) {
	self[0].v = value;
	let $m = null;
	if (scheduler[1].v === 0) {
		for (const subscriber of self[1].v) {
			subscriber[1]();
		}
		$m = undefined;
	} else {
		enqueue(self[1].v);
	}
	return $m;
}
function $n(self, transform) {
	const derived = $a(transform($e(self)));
	self[1].v.push([ fresh_id(), () => {
		$h(derived, transform($e(self)));
		return;
	} ]);
	return derived;
}
const next_subscriber_id = __shared_new(0);
const scheduler = [ __shared_new([  ]), __shared_new(0), __shared_new(false) ];
const owner_scope = null;
const first = $a(1);
const second = $a(10);
const outer = $b(first);
const joined = $c(outer);
console.log($e(joined));
$h(first, 2);
console.log($e(joined));
$l(outer, second);
console.log($e(joined));
$h(first, 99);
console.log($e(joined));
$h(second, 11);
console.log($e(joined));
const doubled = $n(joined, (value) => {
	return value * 2;
});
$h(second, 21);
console.log($e(doubled));
