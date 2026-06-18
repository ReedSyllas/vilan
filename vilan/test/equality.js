function h/*eq*/(i, j) {
	const k = [ i, j ];
	let l = null;
	if (k[0][0] === 0 && k[1][0] === 0) {
		const m/*x*/ = k[0][1];
		const n/*y*/ = k[1][1];
		l = m/*x*/ === n/*y*/;
	} else if (k[0][0] === 1 && k[1][0] === 1) {
		l = true;
	} else {
		l = false;
	}
	return l;
}
function p/*eq*/(q, r) {
	const s = [ q, r ];
	let t = null;
	if (s[0][0] === 0 && s[1][0] === 0) {
		const u/*x*/ = s[0][1];
		const v/*y*/ = s[1][1];
		t = u/*x*/ === v/*y*/;
	} else if (s[0][0] === 1 && s[1][0] === 1) {
		const w/*x*/ = s[0][1];
		const x/*y*/ = s[1][1];
		t = w/*x*/ === x/*y*/;
	} else {
		t = false;
	}
	return t;
}
function d/*eq*/(e, f) {
	return e[0] === f[0] && e[1] === f[1];
}
const a/*p1*/ = [ 1, 2 ];
const b/*p2*/ = [ 1, 2 ];
const c/*p3*/ = [ 3, 4 ];
console.log(d/*eq*/(a/*p1*/, b/*p2*/));
console.log(d/*eq*/(a/*p1*/, c/*p3*/));
console.log(!(d/*eq*/(a/*p1*/, c/*p3*/)));
const g/*a*/ = [ 0, 5 ];
console.log(h/*eq*/(g/*a*/, [ 0, 5 ]));
console.log(h/*eq*/(g/*a*/, [ 1 ]));
console.log(!(h/*eq*/(g/*a*/, [ 0, 7 ])));
const o/*r*/ = [ 0, 1 ];
console.log(p/*eq*/(o/*r*/, [ 0, 1 ]));
console.log(p/*eq*/(o/*r*/, [ 1, "x" ]));
console.log(5 === 5);
console.log("a" === "b");
console.log(true === true);
