console.log("Basic loop with break");
let a/*i*/ = 0;
while (true) {
	if (a/*i*/ >= 10) {
		break;
	}
	console.log(a/*i*/);
	a/*i*/ = a/*i*/ + 1;
}
console.log("While loop with condition");
let b/*i*/ = 0;
while (b/*i*/ < 10) {
	console.log(b/*i*/);
	b/*i*/ = b/*i*/ + 1;
}
console.log("Basic loop with continue and break");
let c/*i*/ = 0;
while (true) {
	console.log(c/*i*/);
	c/*i*/ = c/*i*/ + 1;
	if (c/*i*/ < 10) {
		continue;
	}
	break;
}
