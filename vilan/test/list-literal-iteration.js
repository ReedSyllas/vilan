function sum(self) {
	return self[0] + self[1];
}
const points = [ [ 1, 2 ], [ 3, 4 ] ];
for (const point of points) {
	console.log(sum(point));
}
const numbers = [ 10, 20, 30 ];
for (const number of numbers) {
	console.log(number);
}
