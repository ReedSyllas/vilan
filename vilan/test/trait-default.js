function compare(self2, b2) {
	let $b = null;
	if (self2[0] < b2[0]) {
		$b = -1;
	} else if (self2[0] > b2[0]) {
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
function $d(self3, b3) {
	let $e = null;
	if (compare(self3, b3) >= 0) {
		$e = self3;
	} else {
		$e = b3;
	}
	return $e;
}
function $f(self4, min, max) {
	return $d($a(self4, max), min);
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
