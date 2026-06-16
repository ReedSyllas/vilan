let a/*a*/ = [ 0, 0 ];
let b/*total*/ = 0;
for (const c/*n*/ of [ 1, 2, 3 ]) {
	let d/*b*/ = structuredClone(a/*a*/);
	d/*b*/[0] = d/*b*/[0] + 1;
	b/*total*/ = b/*total*/ + d/*b*/[0];
}
console.log(b/*total*/);
