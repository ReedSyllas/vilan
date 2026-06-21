function __clone(value) {
	if (Array.isArray(value)) return value.map(__clone);
	if (value instanceof Set) return new Set([ ...value ].map(__clone));
	if (value instanceof Map) return new Map([ ...value ].map(([ k, v ]) => [ __clone(k), __clone(v) ]));
	return value;
}
function __shared_new(value) {
	return { v: value };
}
function todos() {
	const items = $a([  ]);
	const draft = $b("");
	const filter = $b("all");
	const next_id = $c(0);
	const remaining = $d(items, (list) => {
		let open = 0;
		for (const todo of list) {
			if (!(todo[2])) {
				open = open + 1;
			}
		}
		return open;
	});
	const visible = $m($g([ items, filter ]), (_) => {
		const $l = _;
		const list = $l[0];
		const filter2 = $l[1];
		let shown = [  ];
		for (const todo of list) {
			const keep = filter2 === "all" || filter2 === "active" && !(todo[2]) || filter2 === "done" && todo[2];
			if (keep) {
				shown.push(todo);
			}
		}
		return shown;
	});
	const add = () => {
		const label = $q(draft);
		if (label !== "") {
			const id = $r(next_id);
			$f(next_id, id + 1);
			$s(items, (list) => {
				let next = __clone(list);
				next.push([ id, label, false ]);
				return next;
			});
			$u(draft, "");
		}
		return;
	};
	return child(child(child(child(child(child(class2(view("section"), "todos"), text(view("h2"), "Todos")), child(child(class2(view("div"), "row"), bind_value(attr(view("input"), "placeholder", "What needs doing?"), draft)), on(text(view("button"), "Add"), "click", add))), bind_text(view("p"), $x(remaining, (n) => {
		return $w(n) + " remaining";
	}))), child(child(child(class2(view("div"), "filters"), filter_button(filter, "all", "All")), filter_button(filter, "active", "Active")), filter_button(filter, "done", "Done"))), $H(view("ul"), visible, (todo) => {
		return todo[0];
	}, (todo) => {
		return todo_row(items, todo);
	})), show(text(class2(view("p"), "empty"), "Nothing here \u{1f389}"), $K(visible, (list) => {
		return list.length === 0;
	})));
}
function filter_button(filter, value, label) {
	return on(bind_class(text(view("button"), label), $z(filter, (current) => {
		let $y = null;
		if (current === value) {
			$y = "active";
		} else {
			$y = "";
		}
		return $y;
	})), "click", () => {
		return $u(filter, value);
	});
}
function todo_row(items, todo) {
	let $A = null;
	if (todo[2]) {
		$A = "done";
	} else {
		$A = "";
	}
	return child(child(child(class2(view("li"), $A), on(attr(view("input"), "type", "checkbox"), "change", () => {
		return toggle(items, todo[0]);
	})), text(view("span"), todo[1])), on(text(class2(view("button"), "remove"), "\u{2715}"), "click", () => {
		return remove(items, todo[0]);
	}));
}
function toggle(items, id) {
	$B(items, (list) => {
		let next = [  ];
		for (const todo of list) {
			if (todo[0] === id) {
				next.push([ todo[0], todo[1], !(todo[2]) ]);
			} else {
				next.push(todo);
			}
		}
		return next;
	});
}
function remove(items, id) {
	$E(items, (list) => {
		let kept = [  ];
		for (const todo of list) {
			if (todo[0] !== id) {
				kept.push(todo);
			}
		}
		return kept;
	});
}
function to_string(self) {
	return "" + self;
}
function fresh_id() {
	const id = next_subscriber_id.v;
	next_subscriber_id.v = id + 1;
	return id;
}
function view(tag) {
	return [ document.createElement(tag) ];
}
function text(self, content) {
	self[0].textContent = content;
	return self;
}
function class2(self, name) {
	self[0].className = name;
	return self;
}
function attr(self, name, value) {
	self[0].setAttribute(name, value);
	return self;
}
function on(self, event, handler) {
	self[0].addEventListener(event, handler);
	return self;
}
function child(self, child2) {
	self[0].appendChild(child2[0]);
	return self;
}
function bind_text(self, source) {
	const element = __clone(self[0]);
	$v(source, (value) => {
		element.textContent = value;
		return;
	});
	return self;
}
function bind_class(self, source) {
	const element = __clone(self[0]);
	$v(source, (value) => {
		element.className = value;
		return;
	});
	return self;
}
function bind_value(self, signal) {
	const element = __clone(self[0]);
	$v(signal, (value) => {
		element.value = value;
		return;
	});
	element.addEventListener("input", () => {
		$u(signal, element.value);
		return;
	});
	return self;
}
function show(self, condition) {
	const element = __clone(self[0]);
	$O(condition, (visible) => {
		element.hidden = !(visible);
		return;
	});
	return self;
}
function mount(id, view2) {
	document.getElementById(id).appendChild(view2[0]);
}
function app() {
	return todos();
}
function $a(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $b(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $c(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $e(self) {
	return self[0].v;
}
function $f(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		subscriber[1]();
	}
}
function $d(self, transform) {
	const derived = $c(transform($e(self)));
	self[1].v.push([ fresh_id(), () => {
		$f(derived, transform($e(self)));
		return;
	} ]);
	return derived;
}
function $h(self) {
	return self[0].v;
}
function $i(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $j(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		subscriber[1]();
	}
}
function $k(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($h(self));
		return;
	} ]);
	observer($h(self));
	return [ self[1], id ];
}
function $g(sources) {
	const snapshot = () => {
		return sources.map((source) => {
			return $h(source);
		});
	};
	const derived = $i(snapshot());
	sources.map((source) => {
		return $k(source, (_) => {
			$j(derived, snapshot());
			return;
		});
	});
	return derived;
}
function $n(self) {
	return self[0].v;
}
function $o(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $p(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		subscriber[1]();
	}
}
function $m(self, transform) {
	const derived = $o(transform($n(self)));
	self[1].v.push([ fresh_id(), () => {
		$p(derived, transform($n(self)));
		return;
	} ]);
	return derived;
}
function $q(self) {
	return self[0].v;
}
function $r(self) {
	return self[0].v;
}
function $t(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		subscriber[1]();
	}
}
function $s(self, transform) {
	$t(self, transform($e(self)));
}
function $u(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		subscriber[1]();
	}
}
function $v(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($q(self));
		return;
	} ]);
	observer($q(self));
	return [ self[1], id ];
}
function $w(value) {
	return to_string(value);
}
function $x(self, transform) {
	const derived = $b(transform($r(self)));
	self[1].v.push([ fresh_id(), () => {
		$u(derived, transform($r(self)));
		return;
	} ]);
	return derived;
}
function $z(self, transform) {
	const derived = $b(transform($q(self)));
	self[1].v.push([ fresh_id(), () => {
		$u(derived, transform($q(self)));
		return;
	} ]);
	return derived;
}
function $C(self) {
	return self[0].v;
}
function $D(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		subscriber[1]();
	}
}
function $B(self, transform) {
	$D(self, transform($C(self)));
}
function $F(self) {
	return self[0].v;
}
function $G(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		subscriber[1]();
	}
}
function $E(self, transform) {
	$G(self, transform($F(self)));
}
function $J(self) {
	return self[0].v;
}
function $I(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($J(self));
		return;
	} ]);
	observer($J(self));
	return [ self[1], id ];
}
function $H(self, source, key, render) {
	const element = __clone(self[0]);
	$I(source, (list) => {
		element.replaceChildren();
		for (const item of list) {
			element.appendChild(render(item)[0]);
		}
		return;
	});
	return self;
}
function $L(self) {
	return self[0].v;
}
function $M(value) {
	let subscribers = [  ];
	return [ __shared_new(value), __shared_new(subscribers) ];
}
function $N(self, value) {
	self[0].v = value;
	for (const subscriber of self[1].v) {
		subscriber[1]();
	}
}
function $K(self, transform) {
	const derived = $M(transform($L(self)));
	self[1].v.push([ fresh_id(), () => {
		$N(derived, transform($L(self)));
		return;
	} ]);
	return derived;
}
function $P(self) {
	return self[0].v;
}
function $O(self, observer) {
	const id = fresh_id();
	self[1].v.push([ id, () => {
		observer($P(self));
		return;
	} ]);
	observer($P(self));
	return [ self[1], id ];
}
const next_subscriber_id = __shared_new(0);
mount("app", app());
