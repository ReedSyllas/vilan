function to_json(self) {
	return "{\"name\":" + JSON.stringify(self[0]) + "," + "\"members\":" + $k(self[1]) + "," + "\"captain\":" + $l(self[2]) + "}";
}
function from_json(text) {
	return from_json_value(JSON.parse(text));
}
function from_json_value(value) {
	return [ String(value["name"]), $h(value["members"]), $i(value["captain"]) ];
}
function $a(text) {
	let result = [  ];
	for (const element of JSON.parse(text)) {
		result.push(Number(element));
	}
	return result;
}
function $b(self) {
	let result = "[";
	let first = true;
	for (const element of self) {
		if (!(first)) {
			result = result + ",";
		}
		result = result + JSON.stringify(element);
		first = false;
	}
	return result + "]";
}
function $c(text) {
	const value = JSON.parse(text);
	let $d = null;
	if (value === null) {
		$d = [ 1 ];
	} else {
		$d = [ 0, Number(value) ];
	}
	return $d;
}
function $e(self) {
	const $f = self;
	let $g = null;
	if ($f[0] === 0) {
		const value = $f[1];
		$g = JSON.stringify(value);
	} else {
		$g = "null";
	}
	return $g;
}
function $h(value) {
	let result = [  ];
	for (const element of value) {
		result.push(String(element));
	}
	return result;
}
function $i(value) {
	let $j = null;
	if (value === null) {
		$j = [ 1 ];
	} else {
		$j = [ 0, String(value) ];
	}
	return $j;
}
function $k(self) {
	let result = "[";
	let first = true;
	for (const element of self) {
		if (!(first)) {
			result = result + ",";
		}
		result = result + JSON.stringify(element);
		first = false;
	}
	return result + "]";
}
function $l(self) {
	const $m = self;
	let $n = null;
	if ($m[0] === 0) {
		const value = $m[1];
		$n = JSON.stringify(value);
	} else {
		$n = "null";
	}
	return $n;
}
function $o(text) {
	let result = [  ];
	for (const element of JSON.parse(text)) {
		result.push(from_json_value(element));
	}
	return result;
}
function $p(self) {
	let result = "[";
	let first = true;
	for (const element of self) {
		if (!(first)) {
			result = result + ",";
		}
		result = result + to_json(element);
		first = false;
	}
	return result + "]";
}
const nums = $a("[1,2,3]");
console.log($b(nums));
const some = $c("7");
console.log($e(some));
const none = $c("null");
console.log($e(none));
const json = "{\"name\":\"Reds\",\"members\":[\"Ada\",\"Bob\"],\"captain\":\"Ada\"}";
const team = from_json(json);
console.log(to_json(team));
console.log($k(team[1]));
const teams = $o("[" + json + "]");
console.log($p(teams));
