function accumulate(i) {
	return i * 10;
}
function one() {
	return 1;
}
function two() {
	return 2;
}
console.log(two());
console.log(one());
const __s2_m0 = (i) => {
	return accumulate(i);
};
console.log(0 + __s2_m0(0) + __s2_m0(1) + __s2_m0(2) + __s2_m0(3));
const __s3_m0 = (i) => {
	return i + 100;
};
console.log(0 + __s3_m0(0) + __s3_m0(1) + __s3_m0(2));
