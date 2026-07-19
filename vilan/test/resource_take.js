function __option_replace(slot, value) {
	const old = slot.slice();
	slot[0] = 0;
	slot[1] = value;
	slot.length = 2;
	return old;
}
function __option_take(slot) {
	const old = slot.slice();
	slot.length = 1;
	slot[0] = 1;
	return old;
}
function drop(self) {
	console.log("drop " + self[0]);
}
function data_take_replace() {
	let a = [ 0, 5 ];
	const taken = __option_take(a);
	console.log("take-data taken=" + $a(taken, 0) + " left_none=" + $d(a));
	let b = [ 0, 1 ];
	const old = __option_replace(b, 2);
	console.log("replace-data old=" + $a(old, 0) + " now=" + $a(b, 0));
}
function take_resource() {
	let opt = [ 0, [ "taken" ] ];
	try {
		const moved = __option_take(opt);
		try {
			console.log("take-res in-block");
		} finally {
			$f(moved);
		}
		console.log("take-res after-block");
	} finally {
		$j(opt);
	}
}
function conditional_teardown() {
	let full = [ 0, [ "cond" ] ];
	try {
		const $l = __option_take(full);
		let $m = null;
		if ($l[0] === 0) {
			const c = $l[1];
			$m = $h(c);
		} else {
			$m = undefined;
		}
		$m;
		console.log("cond after-some");
		let empty = [ 1 ];
		try {
			const $n = __option_take(empty);
			let $o = null;
			if ($n[0] === 0) {
				const c2 = $n[1];
				$o = $h(c2);
			} else {
				$o = console.log("cond none-arm");
			}
			return $o;
		} finally {
			$p(empty);
		}
	} finally {
		$r(full);
	}
}
function sink(r) {
	try {
		console.log("sink " + r[0]);
	} finally {
		$h(r);
	}
}
function passthrough(r) {
	console.log("passthrough");
	return r;
}
function match_move() {
	const holder = [ 0, [ "held" ] ];
	const $t = holder;
	let $u = null;
	if ($t[0] === 0) {
		const inner = $t[1];
		$u = inner;
	} else {
		$u = [ "default" ];
	}
	const extracted = $u;
	console.log("match extracted " + extracted[0]);
	$h(extracted);
}
function $a(self, fallback) {
	const $b = self;
	let $c = null;
	if ($b[0] === 0) {
		const x = $b[1];
		$c = x;
	} else {
		$c = fallback;
	}
	return $c;
}
function $d(self) {
	const $e = self;
	return $e[0] === 1;
}
function $h($i) {
	drop($i);
}
function $f($g) {
	if ($g[0] === 0) {
		$h($g[1]);
	}
}
function $j($k) {
	if ($k[0] === 0) {
		$h($k[1]);
	}
}
function $p($q) {
	if ($q[0] === 0) {
		$h($q[1]);
	}
}
function $r($s) {
	if ($s[0] === 0) {
		$h($s[1]);
	}
}
data_take_replace();
console.log("--");
take_resource();
console.log("--");
conditional_teardown();
console.log("--");
sink([ "sunk" ]);
console.log("sink returned");
const back = passthrough([ "through" ]);
console.log("passthrough returned");
$h(back);
console.log("--");
match_move();
console.log("--");
const db = [ "immediate" ];
console.log("before drop");
$h(db);
console.log("after drop");
