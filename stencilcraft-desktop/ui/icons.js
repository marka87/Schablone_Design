// Einfarbige Linien-Icons im Stil der VS-Code-Codicons (16 × 16, currentColor).
// Verwendung im HTML: <i class="ic ic-save"></i> – wird beim Start durch das SVG ersetzt.
// Im JS: icon("save") liefert den SVG-Text.
(function () {
    const P = {
        logo: '<path d="M9 1 3 9h4l-1 6 6-8H8l1-6z"/>',
        map: '<path d="M1.5 3.5 5.5 2l5 1.5 4-1.5v10.5l-4 1.5-5-1.5-4 1.5zM5.5 2v10.5M10.5 3.5V14"/>',
        package: '<path d="M8 1.5 14 4.5v7L8 14.5 2 11.5v-7zM2 4.5l6 3 6-3M8 7.5v7"/>',
        window: '<rect x="1.5" y="2.5" width="13" height="11" rx="1"/><path d="M1.5 5.5h13M9 8.5h3.5v3H9z"/>',
        folder: '<path d="M1.5 3.5h5l1.5 1.5h6.5v8.5h-13z"/>',
        save: '<path d="M2.5 2.5h9l2 2v9h-11zM5 2.5v4h5v-4M5 13.5v-4h6v4"/>',
        export: '<path d="M8 10V2M5 5l3-3 3 3M2.5 9v4.5h11V9"/>',
        close: '<path d="m4 4 8 8M12 4l-8 8"/>',
        gear: '<circle cx="8" cy="8" r="2.2"/><path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4"/>',
        chart: '<path d="M2 14h12M4 12V8M7 12V4M10 12V7M13 12V2"/>',
        check: '<path d="M2 8.5 6 12.5 14 4"/>',
        plus: '<path d="M8 2.5v11M2.5 8h11"/>',
        plug: '<path d="M6 1.5v3M10 1.5v3M4 4.5h8v3a4 4 0 0 1-8 0zM8 11.5v3"/>',
        calc: '<rect x="3" y="1.5" width="10" height="13" rx="1"/><path d="M5.5 4.5h5M5.5 8h1M9.5 8h1M5.5 11h1M9.5 11h1"/>',
        search: '<circle cx="6.5" cy="6.5" r="4.5"/><path d="m10 10 4.5 4.5"/>',
        layers: '<path d="M8 2 14.5 5.5 8 9 1.5 5.5zM1.5 8.5 8 12l6.5-3.5M1.5 11.5 8 15l6.5-3.5"/>',
        eye: '<path d="M1 8s2.6-4.5 7-4.5S15 8 15 8s-2.6 4.5-7 4.5S1 8 1 8z"/><circle cx="8" cy="8" r="2"/>',
        "eye-closed": '<path d="M1 6s2.6 4 7 4 7-4 7-4M3 8.8l-1.3 1.7M13 8.8l1.3 1.7M8 10v2.2M5.3 9.6l-.8 2M10.7 9.6l.8 2"/>',
        refresh: '<path d="M13.5 8A5.5 5.5 0 1 1 11.6 3.8M13.5 2v3.5H10"/>',
        inbox: '<path d="M1.5 9.5h4l1 2h3l1-2h4M1.5 9.5 3.5 3h9l2 6.5V14h-13z"/>',
        ruler: '<path d="m1.5 11 9.5-9.5 3.5 3.5L5 14.5zM4.5 8l1.5 1.5M6.5 6l1 1M8.5 4l1.5 1.5"/>',
        trash: '<path d="M2.5 4h11M6 4V2.5h4V4M4 4l.7 10h6.6L12 4"/>',
        edit: '<path d="m10.5 2.5 3 3L5 14H2v-3z"/>',
        warning: '<path d="M8 1.5 15 14H1zM8 6v4M8 11.5v1"/>',
        sparkle: '<path d="M8 1.5v4M8 10.5v4M1.5 8h4M10.5 8h4M3.5 3.5l2 2M10.5 10.5l2 2M3.5 12.5l2-2M10.5 5.5l2-2"/>',
        split: '<rect x="2" y="2" width="12" height="12" rx="1"/><path d="M8 2v12M2 8h12"/>',
        shrink: '<rect x="2" y="2" width="12" height="12" rx="1"/><rect x="5" y="5" width="6" height="6"/>',
        library: '<path d="M2 2.5h2.5v11H2zM6 2.5h2.5v11H6zM10 3l2.4-.6 2.3 10.6-2.4.6z"/>',
        tools: '<path d="M10 2.5a3 3 0 0 0-3.4 4L2 11.1 4.9 14l4.6-4.6A3 3 0 0 0 13.5 6l-2 .5-1-1 .5-2z"/>',
        target: '<circle cx="8" cy="8" r="6"/><circle cx="8" cy="8" r="2.5"/>',
        file: '<path d="M3.5 1.5h6l3 3v10h-9zM9.5 1.5v3h3"/>',
        info: '<circle cx="8" cy="8" r="6.5"/><path d="M8 7v4.5M8 4.5v1"/>',
        zap: '<path d="M9 1 3 9h4l-1 6 6-8H8l1-6z"/>',
        microscope: '<path d="M5 2h3v3H5zM6.5 5v5M4 14h9M10.5 14a4 4 0 0 0-2-7"/>',
    };
    window.icon = function (name, cls = "") {
        const p = P[name] || P.info;
        return `<svg class="ic-svg ${cls}" viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${p}</svg>`;
    };
    window.applyIcons = function (root = document) {
        root.querySelectorAll("i.ic").forEach(el => {
            if (el.dataset.done) return;
            const name = [...el.classList].find(c => c.startsWith("ic-"))?.slice(3);
            if (!name) return;
            el.innerHTML = window.icon(name);
            el.dataset.done = "1";
        });
    };
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", () => window.applyIcons());
    else window.applyIcons();
})();
