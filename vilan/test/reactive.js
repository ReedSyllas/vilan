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
function new2() {
	return [ __shared_new([  ]) ];
}
function $a(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $c(self) {
	return self[0].v;
}
function $d(self, value) {
	self[0].v = value;
	let $e = null;
	if (scheduler[1].v === 0) {
		for (const subscriber of self[1].v) {
			subscriber[1]();
		}
		$e = undefined;
	} else {
		enqueue(self[1].v);
	}
	return $e;
}
function $b(self, transform) {
	const derived = $a(transform($c(self)));
	self[1].v.push([ fresh_id(), () => {
		$d(derived, transform($c(self)));
		return;
	} ]);
	return derived;
}
function $f(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($c(self));
		return;
	} ]);
	observer($c(self));
	return [ self[1], id ];
}
function $g(self, item) {
	self[0].v.push(() => {
		dispose(item);
		return;
	});
	return item;
}
function $h(self, transform) {
	$d(self, transform($c(self)));
}
const next_subscriber_id = __shared_new(0);
const scheduler = [ __shared_new([  ]), __shared_new(0), __shared_new(false) ];
const owner = new2();
const count = $a(0);
const doubled = $b(count, (n) => {
	return n * 2;
});
$g(owner, $f(doubled, (n) => {
	return console.log(n);
}));
$d(count, 1);
$h(count, (n) => {
	return n + 4;
});
console.log($c(doubled));
$g(owner, $f(count, (n) => {
	return console.log(n);
}));
$d(count, 20);
