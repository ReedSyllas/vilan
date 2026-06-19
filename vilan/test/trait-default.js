function compare(self, b) {
	let $b = null;
	if (self[0] < b[0]) {
		$b = -1;
	} else if (self[0] > b[0]) {
		$b = 1;
	} else {
		$b = 0;
	}
	return $b;
}
function $a(self, b) {
	let $c = null;
	if (compare(self, b) <= 0) {
		$c = self;
	} else {
		$c = b;
	}
	return $c;
}
function $d(self, b) {
	let $e = null;
	if (compare(self, b) >= 0) {
		$e = self;
	} else {
		$e = b;
	}
	return $e;
}
function $f(self, min, max) {
	return $d($a(self, max), min);
}
const low = [ 3 ];
const high = [ 7 ];
console.log($a(low, high));
console.log($d(low, high));
const below = [ 1 ];
const above = [ 9 ];
const within = [ 5 ];
console.log($f(below, low, high));
console.log($f(above, low, high));
console.log($f(within, low, high));
