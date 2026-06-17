function a/*classify*/(b) {
	const c = b;
	let d = null;
	if (c === "quit") {
		d = "leaving";
	} else if (c === "y") {
		d = "affirmative";
	} else if (c === "") {
		d = "affirmative";
	} else {
		d = "other";
	}
	return d;
}
function e/*describe*/(f) {
	const g = f;
	let h = null;
	if (g[0] === 0) {
		h = "quit";
	} else {
		h = "finished";
	}
	return h;
}
function i/*temperature*/(j) {
	const k = j;
	let l = null;
	if (k <= 2) {
		l = "very hot";
	} else if (k <= 10) {
		l = "warm";
	} else {
		l = "cold";
	}
	return l;
}
console.log(a/*classify*/("quit"));
console.log(a/*classify*/("y"));
console.log(a/*classify*/(""));
console.log(a/*classify*/("maybe"));
console.log(e/*describe*/([ 0 ]));
console.log(e/*describe*/([ 1 ]));
console.log(i/*temperature*/(1));
console.log(i/*temperature*/(7));
console.log(i/*temperature*/(40));
