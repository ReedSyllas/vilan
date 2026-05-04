function /* new */ e() {
	return { height: 30, type: /* RECTANGLE */ c, left: 10, width: 70, name: "Rectangle", top: 20 };
}
const /* CIRCLE */ a = 1;
const /* RECTANGLE */ c = 0;
const /* shape */ d = /* new */ e();
if (/* shape */ d.type === /* RECTANGLE */ c) {
	console.log("shape is a rectangle");
} else if (/* shape */ d.type === /* CIRCLE */ a) {
	console.log("shape is a circle");
} else {
	console.log("shape is not a rectangle or circle");
}
console.log(/* shape */ d);
let g = null;
if (/* shape */ d.type === /* RECTANGLE */ c) {
	g = 1;
} else {
	g = 2;
}
const /* test */ f = g;
console.log(/* test */ f);
