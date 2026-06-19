function bump(xs2) {
	xs2.push(1);
	return xs2.length;
}
let xs = [  ];
const a = bump(xs);
bump(xs);
console.log(a);
console.log(xs.length);
