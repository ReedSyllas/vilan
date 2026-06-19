let names = [  ];
names.push("Anna");
names.push("James");
names.push("Roger");
for (const name of names) {
	console.log(name);
}
let numbers = [  ];
numbers.push(1);
numbers.push(2);
numbers.push(3);
numbers.push(4);
for (const number of numbers) {
	if (number === 3) {
		continue;
	}
	if (number === 4) {
		break;
	}
	console.log(number);
}
let count = 0;
for (const _ of names) {
	count = count + 1;
}
console.log(count);
