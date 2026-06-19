function classify(s) {
	const $a = s;
	let $b = null;
	if ($a === "quit") {
		$b = "leaving";
	} else if ($a === "y") {
		$b = "affirmative";
	} else if ($a === "") {
		$b = "affirmative";
	} else {
		$b = "other";
	}
	return $b;
}
function describe(signal) {
	const $c = signal;
	let $d = null;
	if ($c[0] === 0) {
		$d = "quit";
	} else {
		$d = "finished";
	}
	return $d;
}
function temperature(distance) {
	const $e = distance;
	let $f = null;
	if ($e <= 2) {
		$f = "very hot";
	} else if ($e <= 10) {
		$f = "warm";
	} else {
		$f = "cold";
	}
	return $f;
}
console.log(classify("quit"));
console.log(classify("y"));
console.log(classify(""));
console.log(classify("maybe"));
console.log(describe([ 0 ]));
console.log(describe([ 1 ]));
console.log(temperature(1));
console.log(temperature(7));
console.log(temperature(40));
