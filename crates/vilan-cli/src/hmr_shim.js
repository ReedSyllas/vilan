// vilan dev runtime (HMR) — prepended to browser-leg bundles by an HMR-active
// `vilan run --watch` (hmr.md §2/§3). Plain ES2020, no dependencies. The port,
// this build's version, and this leg's bundle name are template-substituted at
// write time. It installs a `window.__VILAN_HMR__` singleton (a re-evaluated
// bundle reuses it), defines the instrumentation globals the compiled bundle
// calls (`__hmr_adopt*`/`__hmr_expose`, hmr.md §5) plus the `std::dev` host
// globals (`__hmr_register_teardown`/`__hmr_stash`/`__hmr_take`), and reacts to
// the dev channel: live-reload, CSS hot-swap, an error overlay, and the
// state-preserving `swap` (hmr.md §3/§4).
(function () {
    // Singleton guard — a re-evaluated bundle (the swap's `import()`) reuses the
    // live instance and must not open a second EventSource or reset the registry.
    if (window.__VILAN_HMR__) {
        return;
    }
    var PORT = __VILAN_HMR_PORT__;
    var VERSION = __VILAN_HMR_VERSION__;
    var BUNDLE = "__VILAN_HMR_BUNDLE__";

    // Swap state (hmr.md §3/§4). Held in this closure AND on the singleton, so
    // the globals below and the swap protocol share one view; `seed` and
    // `exposed` are mutated in place (never reassigned) to keep both in sync.
    var exposed = {}; // key -> { fp, getter } — the live bindings to capture.
    var seed = {}; // key -> { fp, value } — last capture, consulted on adopt.
    var teardowns = []; // cleanups run once, before the next bundle evaluates.
    var userStash = {}; // "user:"-prefixed app carryover (std::dev stash/take).

    var singleton = {
        port: PORT,
        version: VERSION,
        exposed: exposed,
        seed: seed,
        teardowns: teardowns,
        userStash: userStash,
        take: function (key) {
            var slot = "user:" + key;
            // The `Option` runtime encoding: `[0, value]` = Some, `[1]` = None.
            return Object.prototype.hasOwnProperty.call(userStash, slot)
                ? [0, userStash[slot]]
                : [1];
        },
        swap: swap,
    };
    window.__VILAN_HMR__ = singleton;

    // A binding whose fingerprint changed reinitializes fresh (§10(b)); noted
    // once per adopt call — a module binding's initializer runs once per bundle
    // evaluation, so that is once per key per swap.
    function note(key) {
        if (typeof console !== "undefined" && console.info) {
            console.info("[vilan] hmr: `" + key + "` changed shape — reinitialized");
        }
    }

    // --- Instrumentation globals (hmr.md §5), called by the emitted bundle. ---
    // Assigned to `globalThis` so the bundle's module-scoped top level resolves
    // them as free names. `__hmr_active` is a per-bundle transformer helper, not
    // one of these — the std hooks that guard on it work with no shim too.
    globalThis.__hmr_adopt = function (key, fp, thunk) {
        var entry = seed[key];
        if (entry) {
            if (entry.fp === fp) {
                return entry.value;
            }
            note(key);
        }
        return thunk();
    };
    // A signal/shared binding always builds a FRESH cell (old subscribers must
    // die); on a fingerprint-matching seed hit its payload is written in — the
    // value carries, the identity does not. `Signal` is `[{v},{v:subs}]`
    // (payload at `[0].v`), `Shared` is `{v}` (payload at `.v`).
    globalThis.__hmr_adopt_signal = function (key, fp, thunk) {
        var cell = thunk();
        var entry = seed[key];
        if (entry) {
            if (entry.fp === fp) {
                cell[0].v = entry.value;
            } else {
                note(key);
            }
        }
        return cell;
    };
    globalThis.__hmr_adopt_shared = function (key, fp, thunk) {
        var cell = thunk();
        var entry = seed[key];
        if (entry) {
            if (entry.fp === fp) {
                cell.v = entry.value;
            } else {
                note(key);
            }
        }
        return cell;
    };
    globalThis.__hmr_expose = function (key, fp, getter) {
        exposed[key] = { fp: fp, getter: getter };
    };
    // std::dev host globals — only reached behind an `hmr_active()` guard.
    globalThis.__hmr_register_teardown = function (cleanup) {
        teardowns.push(cleanup);
    };
    globalThis.__hmr_stash = function (key, value) {
        userStash["user:" + key] = value;
    };
    globalThis.__hmr_take = function (key) {
        return singleton.take(key);
    };

    // --- The swap protocol (hmr.md §3). ---
    // Swaps are serialized on a promise chain: a `swap` that arrives while a
    // prior `import()` is still pending would otherwise capture from the
    // already-cleared registry (empty seed) and mount over an un-torn-down
    // page. Chaining makes the second capture see the first bundle's
    // re-registered state.
    var swapChain = Promise.resolve();
    function swap(bundleText) {
        swapChain = swapChain.then(function () {
            return performSwap(bundleText);
        });
        return swapChain;
    }

    function performSwap(bundleText) {
        // (1) Capture — snapshot every exposed binding into the seed (a throwing
        // getter skips its key: fresh init), plus scroll and focus.
        var captured = {};
        for (var key in exposed) {
            if (!Object.prototype.hasOwnProperty.call(exposed, key)) {
                continue;
            }
            try {
                captured[key] = { fp: exposed[key].fp, value: exposed[key].getter() };
            } catch (error) {
                // A throwing getter leaves its binding unseeded — fresh init.
            }
        }
        // Refill `seed` in place so the globals and singleton keep their view.
        for (var stale in seed) {
            if (Object.prototype.hasOwnProperty.call(seed, stale)) {
                delete seed[stale];
            }
        }
        Object.assign(seed, captured);
        var scroll = captureScroll();
        var focus = captureFocus();

        // (2) Teardown — run and clear the list (each isolated), then clear the
        // registry so the next bundle re-registers into an empty one.
        var pending = teardowns.slice();
        teardowns.length = 0;
        for (var index = 0; index < pending.length; index++) {
            try {
                pending[index]();
            } catch (error) {
                // Isolate: one bad teardown must not strand the rest.
            }
        }
        for (var live in exposed) {
            if (Object.prototype.hasOwnProperty.call(exposed, live)) {
                delete exposed[live];
            }
        }

        // (3) Evaluate — import the new bundle as a module (top-level `let` is
        // module-scoped, so old and new bindings never collide).
        var url;
        try {
            url = URL.createObjectURL(new Blob([bundleText], { type: "text/javascript" }));
        } catch (error) {
            reload();
            return Promise.resolve();
        }
        return import(url)
            .then(function () {
                try {
                    URL.revokeObjectURL(url);
                } catch (error) {
                    // A stub URL may not revoke — harmless.
                }
                // (4) Restore scroll/focus best-effort — skip what no longer fits.
                restoreScroll(scroll);
                restoreFocus(focus);
            })
            .catch(function (error) {
                // (5) Teardown already ran — don't limp; reload to a clean boot.
                reload();
            });
    }

    // Host-continuity capture/restore — every host API guarded with `typeof` so
    // the node DOM stub (which lacks most of them) survives.
    function captureScroll() {
        if (typeof window === "undefined") {
            return null;
        }
        return { x: window.scrollX || 0, y: window.scrollY || 0 };
    }
    function restoreScroll(scroll) {
        if (scroll && typeof window !== "undefined" && typeof window.scrollTo === "function") {
            window.scrollTo(scroll.x, scroll.y);
        }
    }
    function captureFocus() {
        if (typeof document === "undefined") {
            return null;
        }
        var active = document.activeElement;
        if (!active || !active.id) {
            return null;
        }
        var info = { id: active.id };
        if (typeof active.selectionStart === "number") {
            info.selectionStart = active.selectionStart;
            info.selectionEnd = active.selectionEnd;
        }
        return info;
    }
    function restoreFocus(focus) {
        if (!focus || typeof document === "undefined") {
            return;
        }
        var element = document.getElementById(focus.id);
        if (!element) {
            return;
        }
        if (typeof element.focus === "function") {
            element.focus();
        }
        if (
            typeof focus.selectionStart === "number" &&
            typeof element.setSelectionRange === "function"
        ) {
            try {
                element.setSelectionRange(focus.selectionStart, focus.selectionEnd);
            } catch (error) {
                // A non-text element rejects a selection range — ignore.
            }
        }
    }

    function reload() {
        if (typeof location !== "undefined" && typeof location.reload === "function") {
            location.reload();
        }
    }

    var OVERLAY_ID = "__vilan_hmr_overlay__";
    // A source line that names a location (`app.vl:12:5`) — styled as a distinct
    // accent line in the overlay, and counted for the header badge.
    var LOCATION_LINE = /:\d+:\d+(\s|$)/;

    function removeOverlay() {
        var existing = document.getElementById(OVERLAY_ID);
        if (existing) {
            existing.remove();
        }
    }

    // The error overlay (hmr.md §2): a dark-translucent backdrop over a slim
    // panel — a header bar ("vilan — build failed" + an error count), the REAL
    // compiler diagnostics in a monospace block with each `file:line:col` on its
    // own accent line and a red left-rule, and a "clears on next save" hint. The
    // terminal stays authoritative; this is the copy for the eyes on the browser.
    // Dependency-free, ES2020, no fonts fetched. Every string is set via
    // `textContent`, so a diagnostic containing `<`/`>` can never inject markup.
    function showOverlay(message) {
        removeOverlay();
        message = message || "build failed — see the terminal";
        var lines = message.split("\n");
        var count = 0;
        for (var i = 0; i < lines.length; i++) {
            if (LOCATION_LINE.test(lines[i])) {
                count += 1;
            }
        }

        var backdrop = document.createElement("div");
        backdrop.id = OVERLAY_ID;
        backdrop.style.cssText =
            "position:fixed;inset:0;z-index:2147483647;overflow:auto;margin:0;padding:32px;" +
            "background:rgba(12,12,16,0.86);color:#e6e6e6;box-sizing:border-box;" +
            "font:13px/1.55 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;";

        var panel = document.createElement("div");
        panel.style.cssText =
            "max-width:920px;margin:0 auto;background:#17171c;border:1px solid #33333c;" +
            "border-left:4px solid #e5484d;border-radius:6px;overflow:hidden;" +
            "box-shadow:0 12px 40px rgba(0,0,0,0.5);";

        var header = document.createElement("div");
        header.style.cssText =
            "display:flex;align-items:center;justify-content:space-between;gap:12px;" +
            "padding:12px 16px;background:#1e1e24;border-bottom:1px solid #2c2c34;";
        var title = document.createElement("span");
        title.style.cssText = "color:#ff6169;font-weight:600;letter-spacing:0.02em;";
        title.textContent = "vilan — build failed";
        header.appendChild(title);
        if (count > 0) {
            var badge = document.createElement("span");
            badge.style.cssText =
                "color:#f0b5b7;background:#3a1e20;border-radius:10px;padding:2px 10px;font-size:12px;";
            badge.textContent = count === 1 ? "1 error" : count + " errors";
            header.appendChild(badge);
        }
        panel.appendChild(header);

        var body = document.createElement("div");
        body.style.cssText = "padding:8px 16px 14px;white-space:pre-wrap;word-break:break-word;";
        for (var j = 0; j < lines.length; j++) {
            var line = lines[j];
            var row = document.createElement("div");
            if (LOCATION_LINE.test(line)) {
                row.style.cssText = "color:#7fd0ff;font-weight:600;margin-top:12px;";
            } else if (/^\s*note:/.test(line)) {
                row.style.cssText = "color:#d6a25f;";
            } else {
                row.style.cssText = "color:#e6e6e6;";
            }
            // A blank line renders as vertical space (a non-breaking space keeps
            // the empty div's height).
            row.textContent = line.length ? line : " ";
            body.appendChild(row);
        }
        panel.appendChild(body);

        var hint = document.createElement("div");
        hint.style.cssText =
            "padding:10px 16px;background:#1e1e24;border-top:1px solid #2c2c34;" +
            "color:#8a8a95;font-size:12px;";
        hint.textContent = "Fixed on next save — this clears on the next successful build.";
        panel.appendChild(hint);

        backdrop.appendChild(panel);
        (document.body || document.documentElement).appendChild(backdrop);
    }

    // A `css` event swaps stylesheets without a reload: bump a cache-busting
    // query so the browser refetches the sidecar. `asset` (when the CLI names it)
    // is the changed sidecar's filename (`client.css`) — bump only the <link>
    // whose href IS that file (hmr.md §2), so a multi-browser-leg workspace
    // refreshes exactly the stylesheet that changed; with no name (an older CLI),
    // bump every stylesheet. The buster is a LOCAL counter, not the build version
    // — css-only rounds deliberately don't bump the version (a bump without a
    // bundle rewrite would send fresh tabs into a reload loop), so consecutive
    // css edits would otherwise produce the same URL and skip the refetch.
    var cssBump = 0;
    function bumpStylesheets(asset) {
        cssBump += 1;
        var links = document.querySelectorAll('link[rel="stylesheet"]');
        for (var index = 0; index < links.length; index++) {
            var link = links[index];
            var base = link.href.split("?")[0];
            if (asset && !(base === asset || base.endsWith("/" + asset))) {
                continue;
            }
            link.href = base + "?v=" + VERSION + "-" + cssBump;
        }
    }

    // A staleness signal (a `swap` event, or a `connected` whose version is
    // ahead of ours): fetch this leg's current bundle from the dev channel —
    // which always serves the fresh dist bytes — and run the swap protocol. On
    // success the singleton's version advances so later `connected` checks
    // agree. A fetch failure warns and WAITS (the next event retries): it must
    // never reload, because the page's own server may serve a bundle it read
    // once at boot — reloading re-fetches that stale bundle, whose shim sees
    // the same version gap and reloads again, forever. The dev channel, not
    // the page reload, is the only sure route to current bytes.
    function fetchAndSwap(version) {
        return fetch("http://127.0.0.1:" + PORT + "/bundle/" + BUNDLE + ".js")
            .then(function (response) {
                return response.text();
            })
            .then(function (text) {
                var result = swap(text);
                if (result && typeof result.then === "function") {
                    return result.then(function () {
                        singleton.version = version;
                    });
                }
                singleton.version = version;
            })
            .catch(function (error) {
                if (typeof console !== "undefined" && console.warn) {
                    console.warn("[vilan] hmr: could not fetch the current bundle; waiting for the next event", error);
                }
            });
    }

    // One dev-channel event. Exposed on the singleton so the node-stub e2e can
    // drive the real event path (EventSource is absent under the stub). Returns
    // the action's promise where there is one, so a test can await completion.
    function handleEvent(data) {
        // Any non-error event clears a lingering overlay.
        if (data.kind !== "error") {
            removeOverlay();
        }
        switch (data.kind) {
            case "connected":
                // Sent on every (re)connect with the channel's current version.
                // A gap means this page runs a stale bundle (the common serving
                // idiom reads dist once at server boot) or missed swaps while
                // disconnected — either way, the heal is a swap from the dev
                // channel, NEVER a reload (hmr.md §2; a reload re-fetches the
                // stale bundle and loops).
                if (data.version !== singleton.version) {
                    return fetchAndSwap(data.version);
                }
                break;
            case "swap":
                return fetchAndSwap(data.version);
            case "reload":
                reload();
                break;
            case "css":
                bumpStylesheets(data.asset);
                break;
            case "error":
                showOverlay(data.message);
                break;
        }
    }
    singleton.handleEvent = handleEvent;

    function connect() {
        // Under the node DOM stub there is no EventSource; the e2e drives
        // `window.__VILAN_HMR__.handleEvent(...)` / `.swap(text)` directly.
        if (typeof EventSource === "undefined") {
            return;
        }
        var source = new EventSource("http://127.0.0.1:" + PORT + "/events");
        source.onmessage = function (event) {
            var data;
            try {
                data = JSON.parse(event.data);
            } catch (error) {
                return;
            }
            handleEvent(data);
        };
        // On error, EventSource reconnects natively — nothing clever to do.
    }

    connect();
})();
