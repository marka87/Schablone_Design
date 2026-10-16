// =============================================================
// SMT StencilCraft Desktop - Tauri-Bridge
// Leitet alle fetch("/api/...")-Aufrufe der Web-Oberfläche an das native
// Rust-Backend (Command `api_request`) weiter. Die Oberfläche (app.js) ist
// identisch zur Flask-Web-App; Antworten haben dieselbe JSON-Struktur.
// =============================================================
(function () {
    const tauri = window.__TAURI__;
    if (!tauri || !tauri.core) {
        console.warn("[StencilCraft] Tauri-API nicht verfügbar – API-Aufrufe gehen an den Webserver.");
        return;
    }
    const invoke = tauri.core.invoke;
    const nativeFetch = window.fetch.bind(window);

    const jsonResponse = (status, body) =>
        new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

    // Browser-Upload (Drag & Drop / Datei-Input): Dateien als Rohdaten an Rust übergeben,
    // dort temporär ablegen und anschließend wie Pfade importieren.
    async function uploadFormData(form) {
        const files = form.getAll("files").filter((f) => f instanceof File);
        if (files.length === 0) {
            return jsonResponse(400, { status: "error", message: "Keine Dateien hochgeladen" });
        }
        const batch = String(Date.now());
        const paths = [];
        for (const file of files) {
            const data = new Uint8Array(await file.arrayBuffer());
            const path = await invoke("stage_upload", data, {
                headers: { "x-file-name": encodeURIComponent(file.name), "x-batch": batch }
            });
            paths.push(path);
        }
        return callApi("POST", "/api/import_paths", {
            paths,
            side: form.get("side") || "top",
            clear_previous: String(form.get("clear_previous") ?? "true").toLowerCase() === "true"
        });
    }

    async function callApi(method, url, body) {
        try {
            const res = await invoke("api_request", { method, url, body: body ?? null });
            return jsonResponse(res.status, res.body);
        } catch (e) {
            console.error("[StencilCraft API]", method, url, e);
            return jsonResponse(500, { status: "error", message: String(e) });
        }
    }

    window.fetch = async function (input, options = {}) {
        const url = typeof input === "string" ? input : input && input.url;
        if (!url || !url.startsWith("/api/")) {
            return nativeFetch(input, options);
        }
        const method = (options.method || "GET").toUpperCase();
        const raw = options.body;
        if (raw instanceof FormData) {
            return uploadFormData(raw);
        }
        let body = null;
        if (typeof raw === "string" && raw.length > 0) {
            try {
                body = JSON.parse(raw);
            } catch (e) {
                return jsonResponse(400, { status: "error", message: "Ungültiger JSON-Body" });
            }
        }
        return callApi(method, url, body);
    };

    window.STENCILCRAFT_DESKTOP = true;
})();
