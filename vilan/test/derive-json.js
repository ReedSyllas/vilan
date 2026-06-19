function to_json(self) {
	return "{\"x\":" + JSON.stringify(self[0]) + "," + "\"y\":" + JSON.stringify(self[1]) + "}";
}
function to_json2(self) {
	return "{\"name\":" + JSON.stringify(self[0]) + "," + "\"age\":" + JSON.stringify(self[1]) + "," + "\"active\":" + JSON.stringify(self[2]) + "," + "\"home\":" + to_json(self[3]) + "}";
}
const p = [ 1, 2 ];
console.log(to_json(p));
const person = [ "Ada \"A\"", 36, true, [ 3, 4 ] ];
console.log(to_json2(person));
