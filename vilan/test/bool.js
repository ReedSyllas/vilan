function c/*describe*/(d) {
	const e = d;
	let f = null;
	if (e === true) {
		f = "yes";
	} else {
		f = "no";
	}
	return f;
}
const a/*flag*/ = true;
const b/*computed*/ = 3 < 5;
console.log(a/*flag*/);
console.log(a/*flag*/ && b/*computed*/);
console.log(!(a/*flag*/));
console.log(!(9 > 5));
console.log(c/*describe*/(a/*flag*/));
console.log(c/*describe*/(9 > 5));
const g = a/*flag*/;
console.log(g === true);
