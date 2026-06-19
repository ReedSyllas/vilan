function to_json(self) {
	return "{\"x\":" + JSON.stringify(self[0]) + "," + "\"y\":" + JSON.stringify(self[1]) + "}";
}
function from_json_value(value) {
	return [ Number(value["x"]), Number(value["y"]) ];
}
function to_json2(self) {
	return "{\"name\":" + JSON.stringify(self[0]) + "," + "\"age\":" + JSON.stringify(self[1]) + "," + "\"active\":" + JSON.stringify(self[2]) + "," + "\"home\":" + to_json(self[3]) + "}";
}
function from_json(text2) {
	return from_json_value2(JSON.parse(text2));
}
function from_json_value2(value) {
	return [ String(value["name"]), Number(value["age"]), Boolean(value["active"]), from_json_value(value["home"]) ];
}
const p = [ 1, 2 ];
console.log(to_json(p));
const person = [ "Ada \"A\"", 36, true, [ 3, 4 ] ];
const text = to_json2(person);
console.log(text);
const back = from_json(text);
console.log(to_json2(back) === text);
console.log(back[3][1]);
console.log(back[0]);
