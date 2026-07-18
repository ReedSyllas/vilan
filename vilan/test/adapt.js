function __sleep(ms, signal) {
	const sig = signal && signal[0] === 0 ? signal[1] : undefined;
	return new Promise((resolve, reject) => {
		if (sig && sig.aborted) {
			reject(sig.reason);
			return;
		}
		const timer = setTimeout(() => resolve(), ms);
		if (sig) sig.addEventListener("abort", () => {
			clearTimeout(timer);
			reject(sig.reason);
		}, { once: true });
	});
}
async function sleep(ms, $a) {
	await (__sleep(ms, ambient_signal($a)));
}
function ambient_signal($b) {
	const $c = $b;
	let $d = null;
	if ($c[0] === 0) {
		const n = $c[1];
		$d = [ 0, n.signal_of() ];
	} else {
		$d = [ 1 ];
	}
	return $d;
}
function run(f) {
	return f() + 100;
}
async function $e(self, fn) {
	let result = [  ];
	for (const item of [ ...self ]) {
		result.push(await (fn(item)));
	}
	return result;
}
function $f(self, fn) {
	let result = [  ];
	for (const item of self) {
		result.push(fn(item));
	}
	return result;
}
async function $g(f) {
	return await (f()) + 100;
}
async function $h(urls, f) {
	return await ($e(urls, f));
}
(async () => {
	const urls = [ "ab", "cdef" ];
	const ids = await ($e(urls, async (url) => {
		const length = url.length;
		await (sleep(1, [ 1 ]));
		return length;
	}));
	console.log(ids);
	console.log($f(urls, (url) => {
		return url.length;
	}));
	console.log(await ($g(async () => {
		await (sleep(1, [ 1 ]));
		return 7;
	})));
	console.log(run(() => {
		return 1;
	}));
	console.log(await ($h(urls, async (url) => {
		await (sleep(1, [ 1 ]));
		return url.length + 10;
	})));
})();
