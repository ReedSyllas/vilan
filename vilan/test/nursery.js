function __nursery_new() {
	return { children: [] };
}
function __nursery_of(option) {
	return option[0] === 0 ? option[1] : undefined;
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
	let index = 0;
	let childFailed = false;
	while (!bodyFailed && !childFailed && index < n.children.length) {
		try {
			await n.children[index++];
		} catch (error) {
			childFailed = true;
		}
	}
	if (!bodyFailed && !childFailed) return result;
	for (const task of n.children) task.then(null, () => {});
	if (bodyFailed) throw bodyError;
	let winner;
	for (const task of n.children) {
		if (task.rejected && (winner === undefined || task.seq < winner.seq)) winner = task;
	}
	if (winner === undefined) throw new Error("nursery: lost the failing task");
	throw typeof winner.error === "string" ? winner.error + " (in task spawned in " + winner.origin + ")" : winner.error;
}
let __task_seq = 0;
class __Task {
	constructor(run, origin, nursery) {
		this.origin = origin;
		this.observed = false;
		this.owned = !!nursery;
		this.rejected = false;
		this.error = undefined;
		this.seq = 0;
		this.promise = run();
		this.promise.then(null, (error) => {
			this.rejected = true;
			this.error = error;
			this.seq = ++__task_seq;
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
async function sleep(ms) {
	await (new Promise((resolve) => {
		setTimeout(resolve, ms);
		return;
	}));
}
function spawn_step(label, ms, $b) {
	__task(async () => {
		await (sleep(ms));
		console.log(label);
		return;
	}, "spawn_step", __nursery_of($b));
}
async function $c(body) {
	const n = __nursery_new();
	return await ((async ($d) => {
		return await (__nursery_run(n, () => {
			return body(n, $d);
		}));
	})(n));
}
(async () => {
	const value = await ($c((n, $a) => {
		spawn_step("helper", 15, [ 0, $a ]);
		__task(async () => {
			await (sleep(5));
			spawn_step("grandchild", 20, [ 0, $a ]);
			console.log("child");
			return;
		}, "main", $a);
		console.log("body");
		return 7;
	}));
	console.log(value);
})();
