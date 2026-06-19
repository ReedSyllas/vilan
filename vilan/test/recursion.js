function fact(n) {
	let $a = null;
	if (n <= 1) {
		$a = 1;
	} else {
		$a = n * fact(n - 1);
	}
	return $a;
}
function is_even(n2) {
	let $b = null;
	if (n2 === 0) {
		$b = true;
	} else {
		$b = is_odd(n2 - 1);
	}
	return $b;
}
function is_odd(n3) {
	let $c = null;
	if (n3 === 0) {
		$c = false;
	} else {
		$c = is_even(n3 - 1);
	}
	return $c;
}
console.log(fact(5));
console.log(fact(10));
console.log(is_even(10));
console.log(is_odd(7));
