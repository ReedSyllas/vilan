function f/*new*/(g, h, i) {
	return [ g, h, i ];
}
function v/*get_name*/(w) {
	return "" + w[2] + " " + w[0] + " " + w[1];
}
function d/*new*/() {
	return [ [  ] ];
}
function j/*add_car*/(k, l) {
	k[0].push(l);
}
function q/*find_car_by_make*/(r, s) {
	for (const t/*car*/ of r[0]) {
		if (t/*car*/[0] === s) {
			return [ 0, t/*car*/ ];
		}
	}
	return [ 1 ];
}
function x/*place_order*/(y, z) {
	let A/*i*/ = 0;
	let B/*has_car*/ = false;
	for (const C/*item*/ of y[0]) {
		A/*i*/ = A/*i*/ + 1;
		if (C/*item*/ === z) {
			B/*has_car*/ = true;
			break;
		}
	}
	let E = null;
	if (B/*has_car*/) {
		const D/*ordered_cars*/ = [  ];
		D/*ordered_cars*/.push(z);
		E = [ 0, [ D/*ordered_cars*/ ] ];
	} else {
		E = [ 1 ];
	}
	return E;
}
function o/*new*/(p) {
	return [ p, [  ] ];
}
function F/*purchase*/(G, H) {
	G[1].push(H);
}
function b/*mock_dealership*/() {
	let c/*dealership*/ = d/*new*/();
	const e/*car1*/ = f/*new*/("Toyota", "Tacoma", 2024);
	j/*add_car*/(c/*dealership*/, e/*car1*/);
	const m/*car2*/ = f/*new*/("Honda", "Accord", 2021);
	j/*add_car*/(c/*dealership*/, m/*car2*/);
	return c/*dealership*/;
}
let a/*dealership*/ = b/*mock_dealership*/();
const n/*john*/ = o/*new*/("John E. Smith");
const u = q/*find_car_by_make*/(a/*dealership*/, "Honda");
if (u[0] === 0) {
	console.log("Found: " + v/*get_name*/(u[1]));
	F/*purchase*/(n/*john*/, x/*place_order*/(a/*dealership*/, u[1]));
	console.log("Purchased: " + v/*get_name*/(u[1]));
}
