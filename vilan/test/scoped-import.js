function tag() {
	return "alpha";
}
function tag2() {
	return "beta";
}
function from_body() {
	console.log("io from a body import");
}
function shadowing() {
	console.log(tag());
	console.log(tag2());
	console.log(tag());
}
function branches(pick) {
	if (pick) {
		console.log("then-arm import");
	}
	const $a = pick;
	let $b = null;
	if ($a === true) {
		console.log("match-arm import");
		$b = undefined;
	} else {
		$b = undefined;
	}
	return $b;
}
from_body();
shadowing();
branches(true);
const closure = () => {
	console.log("closure import");
	return;
};
closure();
