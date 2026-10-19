// =============================================================
// SMT StencilCraft Desktop - Fenster, Skalierung, Tastenkombinationen
//   Strg+O  ODB++ / Gerber öffnen
//   Strg+E  Gerber-Export
//   1/2/3   Farbmodus: Originalfarben / IPC-Heatmap / Blechdicke
// (M = Messlineal und ESC/C = Zurücksetzen sind bereits in app.js enthalten)
// =============================================================
(function () {
    const T = window.__TAURI__;
    const isGeneratorWindow = new URLSearchParams(location.search).get("view") === "package";
    const label = isGeneratorWindow ? "generator" : "main";

    if (T) {
        document.body.classList.add("is-desktop");
        const invoke = T.core.invoke;

        // --- Ereignisse zwischen den Fenstern ---
        window.scBroadcast = (event, payload) =>
            invoke("broadcast", { event, payload: { ...(payload || {}), source: label } }).catch(() => {});

        T.event.listen("engine-changed", (e) => {
            if (e.payload?.source !== label) window.scOnEngineChanged?.(e.payload);
        });
        T.event.listen("pkg-focus", (e) => {
            if (e.payload?.source === label) return;
            window.setPackageHighlight?.(e.payload.pkg, e.payload.pin ?? null);
        });
        T.event.listen("open-package", (e) => {
            if (isGeneratorWindow) window.scOpenPackage?.(e.payload);
        });
        T.event.listen("close-requested", () => window.scHandleCloseRequest?.());
        T.event.listen("view-closed", (e) => {
            if (e.payload === "generator") {
                window.scGeneratorDetached = false;
                document.getElementById("btnDetachGenerator")?.classList.remove("active");
            }
        });

        // --- Stencil Generator in eigenem Fenster ---
        window.scFocusGenerator = () => invoke("open_view_window", { view: "package" }).catch(() => {});
        const btnDetach = document.getElementById("btnDetachGenerator");
        if (isGeneratorWindow) {
            btnDetach?.classList.add("hidden");
        } else {
            invoke("view_window_open", { view: "package" }).then((open) => {
                window.scGeneratorDetached = Boolean(open);
                btnDetach?.classList.toggle("active", Boolean(open));
            }).catch(() => {});
            btnDetach?.addEventListener("click", async () => {
                await invoke("open_view_window", { view: "package" });
                window.scGeneratorDetached = true;
                btnDetach.classList.add("active");
                document.getElementById("btnViewBoard")?.click();
            });
            // Tab "Stencil Generator" holt bei getrenntem Fenster dieses nach vorne
            document.getElementById("btnViewPackage")?.addEventListener("click", (e) => {
                if (window.scGeneratorDetached) {
                    e.stopImmediatePropagation();
                    window.scFocusGenerator();
                }
            }, { capture: true });
        }

        // --- UI-Skalierung (gespeichert, Auto nach Bildschirmbreite) ---
        const sel = document.getElementById("selectUiScale");
        let currentZoom = 1;
        const autoZoom = () => {
            // Bildschirmbreite in CSS-Pixeln bei 100 % (Windows-Skalierung bereits enthalten)
            const cssWidthAt100 = screen.availWidth * currentZoom;
            const z = Math.round(Math.min(1.15, Math.max(0.8, cssWidthAt100 / 2000)) * 20) / 20;
            return z;
        };
        const applyScale = async (value) => {
            const z = value === "auto" ? autoZoom() : parseFloat(value);
            if (!Number.isFinite(z)) return;
            currentZoom = z;
            await invoke("set_ui_zoom", { factor: z }).catch(() => {});
        };
        const saved = localStorage.getItem("sc.uiScale") || "auto";
        if (sel) {
            sel.value = saved;
            sel.addEventListener("change", () => {
                localStorage.setItem("sc.uiScale", sel.value);
                applyScale(sel.value);
            });
        }
        invoke("get_ui_zoom").then((z) => {
            currentZoom = z || 1;
            applyScale(saved);
        }).catch(() => applyScale(saved));
    }

    // --- Tastenkombinationen ---
    window.addEventListener("keydown", (e) => {
        const tag = document.activeElement ? document.activeElement.tagName.toLowerCase() : "";
        const typing = tag === "input" || tag === "textarea" || tag === "select" || document.activeElement?.isContentEditable;
        const ctrl = e.ctrlKey || e.metaKey;

        if (ctrl && e.key.toLowerCase() === "o" && !isGeneratorWindow) {
            e.preventDefault();
            triggerWindowsFileDialog(true);
            return;
        }
        if (ctrl && e.key.toLowerCase() === "e") {
            e.preventDefault();
            window.openExportDialog ? window.openExportDialog() : document.getElementById("btnExport")?.click();
            return;
        }
        if (typing || ctrl || e.altKey) return;

        const modes = { "1": "layers", "2": "ipc", "3": "thickness" };
        const mode = modes[e.key];
        if (mode) {
            const s = document.getElementById("colorModeSelect");
            if (s) {
                s.value = mode;
                s.dispatchEvent(new Event("change"));
            }
        }
    });
})();
