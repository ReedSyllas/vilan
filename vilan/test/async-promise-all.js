import { setTimeout } from "node:timers/promises";
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
