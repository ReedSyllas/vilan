function a/*describe*/(b) {
	const c = b;
	let d = null;
	if (c === -1) {
		d = "less";
	} else if (c === 0) {
		d = "equal";
	} else {
		d = "greater";
	}
	return d;
}
console.log(-1);
console.log(1);
console.log(-1 < 0);
console.log(1 < 0);
console.log(0 === 0);
console.log(a/*describe*/(1));
console.log(a/*describe*/(-1));
