function a/*fact*/(b) {
	let c = null;
	if (b <= 1) {
		c = 1;
	} else {
		c = b * a/*fact*/(b - 1);
	}
	return c;
}
function d/*is_even*/(e) {
	let f = null;
	if (e === 0) {
		f = true;
	} else {
		f = g/*is_odd*/(e - 1);
	}
	return f;
}
function g/*is_odd*/(h) {
	let i = null;
	if (h === 0) {
		i = false;
	} else {
		i = d/*is_even*/(h - 1);
	}
	return i;
}
console.log(a/*fact*/(5));
console.log(a/*fact*/(10));
console.log(d/*is_even*/(10));
console.log(g/*is_odd*/(7));
