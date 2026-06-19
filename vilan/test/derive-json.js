function b/*to_json*/(c) {
	return "{\"x\":" + JSON.stringify(c[0]) + "," + "\"y\":" + JSON.stringify(c[1]) + "}";
}
function e/*to_json*/(f) {
	return "{\"name\":" + JSON.stringify(f[0]) + "," + "\"age\":" + JSON.stringify(f[1]) + "," + "\"active\":" + JSON.stringify(f[2]) + "," + "\"home\":" + b/*to_json*/(f[3]) + "}";
}
const a/*p*/ = [ 1, 2 ];
console.log(b/*to_json*/(a/*p*/));
const d/*person*/ = [ "Ada \"A\"", 36, true, [ 3, 4 ] ];
console.log(e/*to_json*/(d/*person*/));
