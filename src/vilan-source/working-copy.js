function create_shape_e() {
	return { height: 30, name: "Rectangle", top: 20, left: 10, type: RECTANGLE_b, width: 70 };
}
const CIRCLE_a = 1;
const RECTANGLE_b = 0;
const shape_d = create_shape_e();
if (shape_d.type === RECTANGLE_b) {
	console.log("shape is a rectangle");
} else if (shape_d.type === CIRCLE_a) {
	console.log("shape is a circle");
} else {
	console.log("shape is not a rectangle or circle");
}
console.log(shape_d);
const a_g = 10;
const b_h = 5;
const sum_f = a_g + b_h;
console.log(sum_f);
