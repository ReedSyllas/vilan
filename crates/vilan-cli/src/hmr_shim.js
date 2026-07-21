// vilan dev runtime (HMR) — prepended to browser-leg bundles by an HMR-active
// `vilan run --watch` (hmr.md §2). Plain ES2020, no dependencies. The port and
// this build's version are template-substituted at write time. S1 delivers
// live-reload + CSS-without-reload + an in-page error overlay; the `swap`
// reaction is a `location.reload()` placeholder that S2 replaces with the real
// state-preserving swap.
(function () {
    // Singleton guard — a re-evaluated bundle reuses the live instance and must
    // not open a second EventSource (hmr.md §2).
    if (window.__VILAN_HMR__) {
        return;
    }
    var PORT = __VILAN_HMR_PORT__;
    var VERSION = __VILAN_HMR_VERSION__;
    window.__VILAN_HMR__ = { port: PORT, version: VERSION };

    var OVERLAY_ID = "__vilan_hmr_overlay__";

    function removeOverlay() {
        var existing = document.getElementById(OVERLAY_ID);
        if (existing) {
            existing.remove();
        }
    }

    function showOverlay(message) {
        removeOverlay();
        var overlay = document.createElement("div");
        overlay.id = OVERLAY_ID;
        overlay.style.cssText =
            "position:fixed;inset:0;z-index:2147483647;background:rgba(0,0,0,0.85);" +
            "color:#e6e6e6;padding:24px;overflow:auto;margin:0;" +
            "font:13px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace;";
        var pre = document.createElement("pre");
        pre.style.cssText = "margin:0;white-space:pre-wrap;word-break:break-word;";
        pre.textContent = message || "build failed — see the terminal";
        overlay.appendChild(pre);
        (document.body || document.documentElement).appendChild(overlay);
    }

    // A `css` event swaps stylesheets without a reload: bump a cache-busting
    // query on every stylesheet <link> so the browser refetches the sidecar.
    // The buster is a LOCAL counter, not the build version — css-only rounds
    // deliberately don't bump the version (a bump without a bundle rewrite
    // would send fresh tabs into a reload loop), so consecutive css edits
    // would otherwise produce the same URL and skip the refetch.
    var cssBump = 0;
    function bumpStylesheets() {
        cssBump += 1;
        var links = document.querySelectorAll('link[rel="stylesheet"]');
        for (var index = 0; index < links.length; index++) {
            var link = links[index];
            var base = link.href.split("?")[0];
            link.href = base + "?v=" + VERSION + "-" + cssBump;
        }
    }

    function connect() {
        var source = new EventSource("http://127.0.0.1:" + PORT + "/events");
        var first = true;
        source.onmessage = function (event) {
            var data;
            try {
                data = JSON.parse(event.data);
            } catch (error) {
                return;
            }
            // The connect-time version message: heal a stale tab (hmr.md §2).
            if (first && data.kind === "connected") {
                first = false;
                if (data.version !== VERSION) {
                    location.reload();
                }
                return;
            }
            first = false;
            // Any non-error event clears a lingering overlay.
            if (data.kind !== "error") {
                removeOverlay();
            }
            switch (data.kind) {
                case "swap":
                    location.reload();
                    break;
                case "reload":
                    location.reload();
                    break;
                case "css":
                    bumpStylesheets();
                    break;
                case "error":
                    showOverlay(data.message);
                    break;
            }
        };
        // On error, EventSource reconnects natively — nothing clever to do.
    }

    connect();
})();
