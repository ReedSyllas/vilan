function d/*eq*/(e, f) {
	const g = [ e, f ];
	let h = null;
	if (g[0][0] === 0 && g[1][0] === 0) {
		const i/*s0*/ = g[0][1];
		const j/*o0*/ = g[1][1];
		h = i/*s0*/ === j/*o0*/;
	} else if (g[0][0] === 1 && g[1][0] === 1) {
		const k/*s0*/ = g[0][1];
		const l/*s1*/ = g[0][2];
		const m/*o0*/ = g[1][1];
		const n/*o1*/ = g[1][2];
		h = k/*s0*/ === m/*o0*/ && l/*s1*/ === n/*o1*/;
	} else if (g[0][0] === 2 && g[1][0] === 2) {
		h = true;
	} else {
		h = false;
	}
	return h;
}
function o/*debug*/(p) {
	const q = p;
	let r = null;
	if (q[0] === 0) {
		const s/*p0*/ = q[1];
		r = "Circle(" + JSON.stringify(s/*p0*/) + ")";
	} else if (q[0] === 1) {
		const t/*p0*/ = q[1];
		const u/*p1*/ = q[2];
		r = "Rect(" + JSON.stringify(t/*p0*/) + ", " + JSON.stringify(u/*p1*/) + ")";
	} else {
		r = "Empty";
	}
	return r;
}
function v/*to_json*/(w) {
	const x = w;
	let y = null;
	if (x[0] === 0) {
		const z/*p0*/ = x[1];
		y = "{\"Circle\":" + JSON.stringify(z/*p0*/) + "}";
	} else if (x[0] === 1) {
		const A/*p0*/ = x[1];
		const B/*p1*/ = x[2];
		y = "{\"Rect\":[" + JSON.stringify(A/*p0*/) + "," + JSON.stringify(B/*p1*/) + "]}";
	} else {
		y = "\"Empty\"";
	}
	return y;
}
const a/*c*/ = [ 0, 3 ];
const b/*r*/ = [ 1, 4, 5 ];
const c/*e*/ = [ 2 ];
console.log(d/*eq*/(a/*c*/, [ 0, 3 ]));
console.log(d/*eq*/(a/*c*/, [ 0, 9 ]));
console.log(d/*eq*/(a/*c*/, b/*r*/));
console.log(d/*eq*/(c/*e*/, [ 2 ]));
console.log(o/*debug*/(a/*c*/));
console.log(o/*debug*/(b/*r*/));
console.log(o/*debug*/(c/*e*/));
console.log(v/*to_json*/(a/*c*/));
console.log(v/*to_json*/(b/*r*/));
console.log(v/*to_json*/(c/*e*/));
