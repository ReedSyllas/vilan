import { setTimeout } from "node:timers/promises";
class __Task {
	constructor(run, origin) {
		this.origin = origin;
		this.observed = false;
		this.promise = run();
		this.promise.then(null, (error) => {
			if (!this.observed) {
				globalThis.setTimeout(() => {
					if (!this.observed) console.error("unhandled task error (spawned in " + this.origin + "): " + String(error));
				}, 0);
			}
		});
	}
	then(onFulfilled, onRejected) {
		this.observed = true;
		return this.promise.then(onFulfilled, onRejected);
	}
}
function __task(run, origin) {
	return new __Task(run, origin);
}
async function delayed(label, ms) {
	await (setTimeout(ms));
	return label;
}
(async () => {
	let tasks = [  ];
	tasks.push(__task(async () => {
		return await (delayed("a", 20));
	}, "main"));
	tasks.push(__task(async () => {
		return await (delayed("b", 10));
	}, "main"));
	tasks.push(__task(async () => {
		return await (delayed("c", 30));
	}, "main"));
	const results = await (Promise.all(tasks));
	for (const result of results) {
		console.log(result);
	}
})();
