function compare(self, b) {
	let $h = null;
	if (self < b) {
		$h = -1;
	} else {
		let $i = null;
		if (self > b) {
			$i = 1;
		} else {
			$i = 0;
		}
		$h = $i;
	}
	return $h;
}
function rem(self, m) {
	return self - Math.trunc(self / m) * m;
}
function rem2(self, m) {
	return self - Math.trunc(self / m) * m;
}
function fract(self) {
	return self - Math.trunc(self);
}
function lerp(self, to, t) {
	return self + (to - self) * t;
}
function to_radians(self) {
	return self * (PI / 180);
}
function to_degrees(self) {
	return self * (180 / PI);
}
function rem3(self, m) {
	return self - Math.trunc(self / m) * m;
}
function is_nan(self) {
	return self !== self;
}
function is_infinite(self) {
	return !(Number.isFinite(self)) && !(is_nan(self));
}
function compare2(self, b) {
	let $c = null;
	if (self < b) {
		$c = -1;
	} else {
		let $d = null;
		if (self > b) {
			$d = 1;
		} else {
			$d = 0;
		}
		$c = $d;
	}
	return $c;
}
function rem4(self, m) {
	return self - Math.trunc(self / m) * m;
}
function rem5(self, m) {
	return self - Math.trunc(self / m) * m;
}
function $b(self, b) {
	let $e = null;
	if (compare2(self, b) <= 0) {
		$e = self;
	} else {
		$e = b;
	}
	return $e;
}
function $a(a, b) {
	return $b(a, b);
}
function $g(self, b) {
	let $j = null;
	if (compare(self, b) >= 0) {
		$j = self;
	} else {
		$j = b;
	}
	return $j;
}
function $f(a, b) {
	return $g(a, b);
}
function $k(a, b) {
	let $l = null;
	if (a <= b) {
		$l = [ a, b ];
	} else {
		$l = [ b, a ];
	}
	return $l;
}
const PI = 3.141592653589793;
const TAU = 6.283185307179586;
const E = 2.718281828459045;
const EPSILON = Math.pow(2, 0 - 52);
const INFINITY = 1 / 0;
const NAN = 0 / 0;
console.log(PI);
console.log(TAU === PI * 2);
console.log(E > 2.718 && E < 2.719);
console.log(EPSILON === Math.pow(2, 0 - 52));
console.log(INFINITY > 0 && is_infinite(INFINITY));
console.log(is_nan(NAN));
console.log($a(3, 9));
console.log($f("ant", "bee"));
const $m = $k(9, 3);
const low = $m[0];
const high = $m[1];
console.log("" + low + " " + high);
console.log(Math.sin(0));
console.log(Math.cos(0));
console.log(Math.atan2(0, 1));
console.log(to_radians(180) === PI);
console.log(to_degrees(PI));
console.log(Math.exp(0));
console.log(Math.log(1));
console.log(Math.log2(8));
console.log(Math.log10(1000));
console.log(Math.cbrt(27));
console.log(Math.hypot(3, 4));
console.log(Math.sign(0 - 5));
console.log(fract(1.5));
console.log(lerp(0, 10, 0.5));
console.log(rem3(7.5, 2));
console.log(Number.isFinite(1.5));
console.log(Number.isFinite(INFINITY));
console.log(Math.abs(0 - 5));
console.log(Math.pow(3, 2));
console.log(Math.min(200, 90));
console.log(Math.max(7, 9));
console.log(rem2(7, 3));
console.log(rem2(0 - 7, 3));
console.log(rem4(250, 7));
console.log(rem5(9, 4));
console.log(rem(3000000000, 7));
console.log(Math.pow(2, 3));
console.log(Math.sqrt(2.25));
console.log(Math.abs(0 - 1.5));
