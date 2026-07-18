async function sleep(ms) {
	await (new Promise((resolve) => {
		setTimeout(resolve, ms);
		return;
	}));
}
function run(f) {
	return f() + 100;
}
async function $a(self, fn) {
	let result = [  ];
	for (const item of [ ...self ]) {
		result.push(await (fn(item)));
	}
	return result;
}
function $b(self, fn) {
	let result = [  ];
	for (const item of self) {
		result.push(fn(item));
	}
	return result;
}
async function $c(f) {
	return await (f()) + 100;
}
async function $d(urls, f) {
	return await ($a(urls, f));
}
(async () => {
	const urls = [ "ab", "cdef" ];
	const ids = await ($a(urls, async (url) => {
		const length = url.length;
		await (sleep(1));
		return length;
	}));
	console.log(ids);
	console.log($b(urls, (url) => {
		return url.length;
	}));
	console.log(await ($c(async () => {
		await (sleep(1));
		return 7;
	})));
	console.log(run(() => {
		return 1;
	}));
	console.log(await ($d(urls, async (url) => {
		await (sleep(1));
		return url.length + 10;
	})));
})();
