function $a(value) {
	return JSON.stringify(value);
}
function $b(value) {
	return JSON.stringify(value);
}
function $c(self) {
	let result = "[";
	let first = true;
	for (const element of self) {
		if (!(first)) {
			result = result + ",";
		}
		result = result + JSON.stringify(element);
		first = false;
	}
	return result + "]";
}
function $d(self) {
	const $e = self;
	let $f = null;
	if ($e[0] === 0) {
		const value = $e[1];
		$f = JSON.stringify(value);
	} else {
		$f = "null";
	}
	return $f;
}
console.log($a(42));
console.log($b("hi"));
const nums = [ 1, 2, 3 ];
console.log($c(nums));
const maybe = [ 0, 7 ];
console.log($d(maybe));
const nothing = [ 1 ];
console.log($d(nothing));
