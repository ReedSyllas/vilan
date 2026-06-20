function to_json(self) {
	return "{\"name\":" + JSON.stringify(self[0]) + "," + "\"members\":" + $m(self[1]) + "," + "\"captain\":" + $n(self[2]) + "}";
}
function from_json(text) {
	return from_json_value(JSON.parse(text));
}
function from_json_value(value) {
	return [ String(value["name"]), $j(value["members"]), $k(value["captain"]) ];
}
function $b(value) {
	let result = [  ];
	for (const element of value) {
		result.push(Number(element));
	}
	return result;
}
function $a(text) {
	return $b(JSON.parse(text));
}
function $c(self) {
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
function $e(value) {
	let $f = null;
	if (value === null) {
		$f = [ 1 ];
	} else {
		$f = [ 0, Number(value) ];
	}
	return $f;
}
function $d(text) {
	return $e(JSON.parse(text));
}
function $g(self) {
	const $h = self;
	let $i = null;
	if ($h[0] === 0) {
		const value = $h[1];
		$i = JSON.stringify(value);
	} else {
		$i = "null";
	}
	return $i;
}
function $j(value) {
	let result = [  ];
	for (const element of value) {
		result.push(String(element));
	}
	return result;
}
function $k(value) {
	let $l = null;
	if (value === null) {
		$l = [ 1 ];
	} else {
		$l = [ 0, String(value) ];
	}
	return $l;
}
function $m(self) {
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
function $n(self) {
	const $o = self;
	let $p = null;
	if ($o[0] === 0) {
		const value = $o[1];
		$p = JSON.stringify(value);
	} else {
		$p = "null";
	}
	return $p;
}
function $r(value) {
	let result = [  ];
	for (const element of value) {
		result.push(from_json_value(element));
	}
	return result;
}
function $q(text) {
	return $r(JSON.parse(text));
}
function $s(self) {
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
console.log($c(nums));
const some = $d("7");
console.log($g(some));
const none = $d("null");
console.log($g(none));
const json = "{\"name\":\"Reds\",\"members\":[\"Ada\",\"Bob\"],\"captain\":\"Ada\"}";
const team = from_json(json);
console.log(to_json(team));
console.log($m(team[1]));
const teams = $q("[" + json + "]");
console.log($s(teams));
