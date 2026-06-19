console.log("Basic loop with break");
let i = 0;
while (true) {
	if (i >= 10) {
		break;
	}
	console.log(i);
	i = i + 1;
}
console.log("While loop with condition");
let i2 = 0;
while (i2 < 10) {
	console.log(i2);
	i2 = i2 + 1;
}
console.log("Basic loop with continue and break");
let i3 = 0;
while (true) {
	console.log(i3);
	i3 = i3 + 1;
	if (i3 < 10) {
		continue;
	}
	break;
}
