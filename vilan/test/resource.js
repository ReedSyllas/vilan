function drop(self) {
	console.log(self[0]);
}
function drop2(self) {
	console.log("pair-body");
}
function locals() {
	const a = [ "a" ];
	try {
		const b = [ "b" ];
		try {
			console.log("locals-body");
		} finally {
			$a(b);
		}
	} finally {
		$a(a);
	}
}
function early(stop) {
	const r = [ "early" ];
	try {
		if (stop) {
			console.log("stopping");
			return;
		}
		console.log("continuing");
	} finally {
		$a(r);
	}
}
function overwrite() {
	let r = [ "old" ];
	try {
		$a(r);
		r = [ "new" ];
		console.log("overwrite-body");
	} finally {
		$a(r);
	}
}
function nested() {
	const pair = [ [ "first" ], [ "second" ] ];
	try {
		console.log("nested-body");
	} finally {
		$c(pair);
	}
}
function containment() {
	const bag = [ [ "bagged" ] ];
	try {
		console.log("containment-body");
	} finally {
		$e(bag);
	}
}
function $a($b) {
	drop($b);
}
function $c($d) {
	drop2($d);
	$a($d[1]);
	$a($d[0]);
}
function $e($f) {
	$a($f[0]);
}
locals();
console.log("--");
early(true);
console.log("--");
early(false);
console.log("--");
overwrite();
console.log("--");
nested();
console.log("--");
containment();
