function __parse_f64(text) {
	const trimmed = text.trim();
	const value = Number(trimmed);
	return trimmed === "" || Number.isNaN(value) ? [ 1 ] : [ 0, value ];
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
	return $e[0] === 0;
}
console.log($a(__parse_f64("3.14"), 0));
console.log($a(__parse_f64("42"), 0));
console.log($a(__parse_f64("-2.5"), 0));
console.log($a(__parse_f64("nope"), -(1)));
console.log($d(__parse_f64("3.14")));
console.log($d(__parse_f64("abc")));
