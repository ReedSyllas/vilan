class __Nursery {
	constructor(parent) {
		this.children = [];
		this.failedTask = undefined;
		this.failWake = undefined;
		this.controller = new AbortController();
		if (parent) {
			const signal = parent.controller.signal;
			if (signal.aborted) this.controller.abort(signal.reason);
			else signal.addEventListener("abort", () => this.controller.abort(signal.reason), { once: true });
		}
	}
	cancel() {
		this.controller.abort();
	}
	__fail(task) {
		if (this.failedTask === undefined) {
			this.failedTask = task;
			this.controller.abort();
			if (this.failWake) this.failWake();
		}
	}
	is_cancelled() {
		return this.controller.signal.aborted;
	}
	signal_of() {
		return this.controller.signal;
	}
}
function __nursery_new(parent) {
	return new __Nursery(parent && parent[0] === 0 ? parent[1] : undefined);
}
function __nursery_of(option) {
	return option[0] === 0 ? option[1] : undefined;
}
function __nursery_is_cancel(error) {
	return !!error && error.name === "AbortError";
}
async function __nursery_run(n, body) {
	let result;
	let bodyError;
	let bodyFailed = false;
	try {
		result = await body();
	} catch (error) {
		bodyFailed = true;
		bodyError = error;
	}
	if (bodyFailed) n.controller.abort();
	const failed = new Promise((resolve) => {
		n.failWake = resolve;
		if (n.failedTask !== undefined) resolve();
	});
	let index = 0;
	while (!bodyFailed && n.failedTask === undefined && index < n.children.length) {
		try {
			await Promise.race([n.children[index], failed]);
		} catch (error) {}
		if (n.failedTask === undefined) index += 1;
	}
	if (!bodyFailed && n.failedTask === undefined) return result;
	for (const task of n.children) task.then(null, () => {});
	if (bodyFailed) throw bodyError;
	const winner = n.failedTask;
	throw typeof winner.error === "string" ? winner.error + " (in task spawned in " + winner.origin + ")" : winner.error;
}
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
class __Task {
	constructor(run, origin, nursery) {
		this.origin = origin;
		this.observed = false;
		this.nursery = nursery;
		this.owned = !!nursery;
		this.rejected = false;
		this.error = undefined;
		this.promise = run();
		this.promise.then(null, (error) => {
			this.rejected = true;
			this.error = error;
			if (this.owned && !__nursery_is_cancel(error)) this.nursery.__fail(this);
			if (!this.observed && !this.owned) {
				globalThis.setTimeout(() => {
					if (!this.observed) console.error("unhandled task error (spawned in " + this.origin + "): " + String(error));
				}, 0);
			}
		});
		if (nursery) nursery.children.push(this);
	}
	then(onFulfilled, onRejected) {
		this.observed = true;
		return this.promise.then(onFulfilled, onRejected);
	}
}
function __task(run, origin, nursery) {
	return new __Task(run, origin, nursery);
}
function ambient_signal($d) {
	const $e = $d;
	let $f = null;
	if ($e[0] === 0) {
		const n = $e[1];
		$f = [ 0, n.signal_of() ];
	} else {
		$f = [ 1 ];
	}
	return $f;
}
async function sleep(ms, $c) {
	await (__sleep(ms, ambient_signal($c)));
}
function spawn_step(label, ms, $b) {
	__task(async () => {
		await (sleep(ms, $b));
		console.log(label);
		return;
	}, "spawn_step", __nursery_of($b));
}
async function $g(body, $h) {
	const n = __nursery_new($h);
	return await ((async ($i) => {
		return await (__nursery_run(n, () => {
			return body(n, $i);
		}));
	})(n));
}
(async () => {
	const value = await ($g((n, $a) => {
		spawn_step("helper", 15, [ 0, $a ]);
		__task(async () => {
			await (sleep(5, [ 0, $a ]));
			spawn_step("grandchild", 20, [ 0, $a ]);
			console.log("child");
			return;
		}, "main", $a);
		console.log("body");
		return 7;
	}, [ 1 ]));
	console.log(value);
})();
