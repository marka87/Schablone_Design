/**
 * SMT Stencil Craft - Interactive CAD Viewer & Modifier
 * Ultra-performantes HTML5 Canvas 2D Rendering mit 60 FPS
 */

// State
const state = {
    bounds: [0, 0, 210, 308], // min_x, min_y, max_x, max_y in mm
    apertures: [],
    pads: [],
    components: [],
    zones: [],
    profile: [],
    fiducials: [],
    stats: null,

    // Viewport Transform (World mm -> Screen Canvas px)
    zoom: 1.0,
    panX: 0,
    panY: 0,
    isDragging: false,
    dragStartX: 0,
    dragStartY: 0,

    // Selection & Modus
    selectedApertureId: null,
    selectedAperture: null,
    highlightIdentical: true,
    colorMode: "layers", // 'layers', 'ipc', 'thickness'

    // Multi-Layer Manager State
    layers: [],
    layerPrimitives: {},
    globalLayerOpacity: 0.85,

    // Zone Drawing Mode
    isDrawingZone: false,
    drawStartWorld: null,
    drawCurrentWorld: null,

    // Legacy fallback UI Toggles
    showPaste: true,
    showPads: true,
    showComponents: true,
    showZones: true,
    showProfile: true,
    showFiducials: true,

    // Calculator & DFM State
    calcUnit: "mil",
    pasteType: "type4",
    minBridge: 0.10,
    bridgeViolations: [],
    pipShape: "rectangle"
};

// DOM Elements
const canvas = document.getElementById("cadCanvas");
const ctx = canvas.getContext("2d");
const container = document.getElementById("canvasContainer");

// -------------------------------------------------------------
// Interactive Ruler & Measurement Tool State
// -------------------------------------------------------------
const measureState = {
    active: false,
    startPoint: null,       // { x, y } in world coordinates (mm)
    currentPoint: null,     // { x, y } in world coordinates (mm)
    hoverSnap: null,        // { x, y, label } if snapped
    measurements: []        // Array of { id, mode: 'board'|'package', start: {x,y}, end: {x,y}, dist, dx, dy }
};

// Initialize
window.addEventListener("DOMContentLoaded", async () => {
    setupCanvas();
    setupEventListeners();
    setupSplitters();
    setupPackageHighlight();
    const view = new URLSearchParams(location.search).get("view");
    if (view === "package") {
        // Eigenes Stencil-Generator-Fenster
        document.body.classList.add("view-package-only");
        document.title = "Stencil Generator – SMT StencilCraft";
        switchWorkspace("package");
    }
    setupProjectUI();
    pkgState.pinsExpanded = localStorage.getItem("sc.pinsOpen") !== "0";
    // Einzelpad-Werkzeuge im Inspector: Auf-/Zu-Zustand merken
    const editTools = document.getElementById("inspEditTools");
    if (editTools) {
        editTools.open = localStorage.getItem("sc.inspEdit") === "1";
        editTools.addEventListener("toggle", () => localStorage.setItem("sc.inspEdit", editTools.open ? "1" : "0"));
    }
    // Grund-Blechdicke
    document.getElementById("btnThickInfo")?.addEventListener("click", (e) => { e.stopPropagation(); openThicknessOverview(); });
    document.addEventListener("click", (e) => {
        if (!e.target.closest?.("#thickInfoMenu")) document.getElementById("thickPopover")?.classList.add("hidden");
    });
    document.getElementById("inputBaseThickness")?.addEventListener("change", async (e) => {
        const um = parseFloat(e.target.value);
        if (!(um >= 30 && um <= 500)) { updateThicknessInput(true); return; }
        await fetch("/api/set_thickness", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ thickness: um / 1000 }) });
        await refreshApertureData();
        await refreshHistory();
        if (typeof pkgInitialized !== "undefined" && pkgInitialized && pkgState.activePkgName) await refreshActivePackage();
        showToast(`Base stencil thickness ${um} µm`, 1800);
    });
    document.getElementById("selectRuleLayout")?.addEventListener("change", (e) => applyRuleLayout(e.target.value));
    applyRuleLayout("right"); // Eigenschaften fest rechts wie PCB-I
    await loadLayers();
    await refreshApertureData();
    fitToScreen();
    await refreshHistory();
    refreshSides();
    if (view !== "package") {
        setInterval(() => fetch("/api/project/autosave", { method: "POST" }).catch(() => {}), 60000);
        offerRecovery();
    }
});

// -------------------------------------------------------------
// Layout: verschiebbare / einklappbare Bereiche (Größen werden gespeichert)
// -------------------------------------------------------------
const PANEL_VARS = {
    layerPanel: "--left-panel-w",
    boardSidebar: "--right-panel-w",
    pkgSidebar: "--pkg-sidebar-w",
    ruleConfigCard: "--rule-card-h",
    // Breite der Regel-Einstellungen, wenn sie seitlich angeordnet sind
    ruleConfigCard_w: "--rule-card-w"
};

function saveLayout() {
    const root = document.documentElement;
    const sizes = {};
    for (const [id, v] of Object.entries(PANEL_VARS)) {
        const val = root.style.getPropertyValue(v);
        if (val) sizes[id] = parseFloat(val);
    }
    const collapsed = Object.keys(PANEL_VARS).filter(id => document.getElementById(id)?.classList.contains("panel-collapsed"));
    localStorage.setItem("sc.layout", JSON.stringify({ sizes, collapsed }));
}

// Regel-Einstellungen oben / rechts / links (wird gespeichert)
function applyRuleLayout(mode) {
    if (!["top", "right", "left"].includes(mode)) mode = "top";
    const main = document.querySelector(".pkg-main-content");
    if (!main) return;
    main.classList.toggle("rules-right", mode === "right");
    main.classList.toggle("rules-left", mode === "left");
    const sp = main.querySelector('.splitter[data-target="ruleConfigCard"]');
    if (sp) {
        sp.classList.toggle("splitter-h", mode === "top");
        sp.classList.toggle("splitter-v", mode !== "top");
        sp.dataset.side = mode === "right" ? "right" : (mode === "left" ? "left" : "top");
    }
    const sel = document.getElementById("selectRuleLayout");
    if (sel) sel.value = mode;
    localStorage.setItem("sc.ruleLayout", mode);
    onLayoutChanged();
}
window.applyRuleLayout = applyRuleLayout;

function onLayoutChanged() {
    resizeCanvas();
    render();
    if (typeof pkgInitialized !== "undefined" && pkgInitialized) {
        resizePkgCanvas();
        renderPkgPreview();
    }
}

function setupSplitters() {
    const root = document.documentElement;
    let saved = {};
    try { saved = JSON.parse(localStorage.getItem("sc.layout") || "{}"); } catch (e) { saved = {}; }
    for (const [id, px] of Object.entries(saved.sizes || {})) {
        if (PANEL_VARS[id] && px > 0) root.style.setProperty(PANEL_VARS[id], `${px}px`);
    }
    for (const id of saved.collapsed || []) document.getElementById(id)?.classList.add("panel-collapsed");

    document.querySelectorAll(".splitter").forEach(sp => {
        const target = document.getElementById(sp.dataset.target);
        if (!target) return;
        sp.addEventListener("mousedown", (e) => {
            e.preventDefault();
            // Ausrichtung zur Laufzeit lesen (Regel-Einstellungen können oben oder seitlich sitzen)
            const horiz = sp.classList.contains("splitter-h");
            const side = sp.dataset.side;
            const sizeVar = (!horiz && PANEL_VARS[`${target.id}_w`]) || PANEL_VARS[target.id];
            target.classList.remove("panel-collapsed");
            const startPos = horiz ? e.clientY : e.clientX;
            const r = target.getBoundingClientRect();
            const start = horiz ? r.height : r.width;
            sp.classList.add("dragging");
            document.body.classList.add("resizing");
            document.body.style.cursor = horiz ? "row-resize" : "col-resize";
            const move = (ev) => {
                const d = (horiz ? ev.clientY : ev.clientX) - startPos;
                const limit = (horiz ? window.innerHeight : window.innerWidth) * 0.65;
                const size = Math.max(140, Math.min(limit, side === "right" ? start - d : start + d));
                root.style.setProperty(sizeVar, `${Math.round(size)}px`);
                onLayoutChanged();
            };
            const up = () => {
                window.removeEventListener("mousemove", move);
                window.removeEventListener("mouseup", up);
                sp.classList.remove("dragging");
                document.body.classList.remove("resizing");
                document.body.style.cursor = "";
                saveLayout();
            };
            window.addEventListener("mousemove", move);
            window.addEventListener("mouseup", up);
        });
        sp.addEventListener("dblclick", () => {
            target.classList.toggle("panel-collapsed");
            saveLayout();
            onLayoutChanged();
        });
    });
}

// -------------------------------------------------------------
// Package-Hervorhebung im Board (alle Instanzen eines Packages, optional ein Pin)
// -------------------------------------------------------------
function setupPackageHighlight() {
    document.getElementById("btnHighlightClear")?.addEventListener("click", () => setPackageHighlight(null));
    document.getElementById("btnHighlightZoom")?.addEventListener("click", zoomToHighlight);
    document.getElementById("chkHighlightDim")?.addEventListener("change", (e) => { state.highlightDim = e.target.checked; render(); });
    document.getElementById("btnHighlightStep")?.addEventListener("click", () => openStepPanel(state.highlightPkg));
    document.getElementById("btnStepClose")?.addEventListener("click", closeStepPanel);
    document.getElementById("stepThick")?.addEventListener("change", () => updateStepProposal());
    document.getElementById("stepGap")?.addEventListener("change", () => updateStepProposal());
    document.getElementById("btnStepCreate")?.addEventListener("click", createProposedZones);
    document.getElementById("btnSuggestZones")?.addEventListener("click", suggestZones);
    document.getElementById("btnMergeZones")?.addEventListener("click", async () => {
        const d = await (await fetch("/api/zones/merge", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" })).json();
        await refreshApertureData();
        await refreshHistory();
        showToast(d.merged ? `${d.merged} zone(s) of equal thickness merged (Ctrl+Z to undo)` : "No overlapping step zones of equal thickness");
    });
}

// Stufenzonen um alle Bauteile eines Packages vorschlagen (Vorschau gestrichelt, dann anlegen)
async function openStepPanel(pkg, thicknessUm = null) {
    if (!pkg) return;
    state.stepPkg = pkg;
    document.getElementById("stepPkg").textContent = pkg;
    document.getElementById("stepPanel").classList.remove("hidden");
    await updateStepProposal(thicknessUm);
}

function closeStepPanel() {
    document.getElementById("stepPanel")?.classList.add("hidden");
    state.zoneProposals = null;
    state.stepPkg = null;
    render();
}

async function updateStepProposal(thicknessUm = null) {
    if (!state.stepPkg) return;
    const gap = parseFloat(document.getElementById("stepGap").value) || 0;
    const d = await (await fetch("/api/zones/propose", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ package: state.stepPkg, merge_gap: gap }) })).json();
    const th = document.getElementById("stepThick");
    if (thicknessUm) th.value = thicknessUm;
    else if (!th.dataset.pkg || th.dataset.pkg !== state.stepPkg) th.value = Math.round(d.thickness * 1000);
    th.dataset.pkg = state.stepPkg;
    state.zoneProposals = d.zones;
    const comps = d.zones.reduce((n, z) => n + z.components.length, 0);
    const foreign = d.zones.reduce((n, z) => n + z.foreign_pads, 0);
    const overl = d.zones.filter(z => z.overlaps_zone).length;
    const base = Math.round((state.stats?.base_thickness || 0.12) * 1000);
    const need = d.needed ? `Recommended ${Math.round(d.t_step * 1000)} µm (base ${base} µm)` : `Pads OK up to ${Math.round(d.t_step * 1000)} µm – no step needed, any thickness`;
    document.getElementById("stepInfo").innerHTML = `${d.zones.length} zone(s) for ${comps} component(s) · ${need}`
        + (foreign ? `<br><span class="warn">${foreign} pads of other components are inside the zones and will also be thinner.</span>` : "")
        + (overl ? `<br><span class="warn">${overl} zone(s) overlap existing step zones.</span>` : "");
    render();
}

async function createProposedZones() {
    if (!state.zoneProposals?.length) return;
    const um = parseFloat(document.getElementById("stepThick").value);
    const d = await (await fetch("/api/zones/create", { method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ zones: state.zoneProposals, thickness: um / 1000, name: `Step ${state.stepPkg} ${um}µm` }) })).json();
    const pkg = state.stepPkg;
    closeStepPanel();
    await refreshApertureData();
    await refreshHistory();
    document.querySelector('.tab-btn[data-tab="tabZones"]')?.click();
    showToast(`${d.created} step zone(s) ${um} µm for ${pkg} created (Ctrl+Z to undo)`);
}

// Vorschläge für alle Packages, die bei der Grunddicke nicht reichen
async function suggestZones() {
    const d = await (await fetch("/api/zones/suggest")).json();
    const box = document.getElementById("zoneSuggestList");
    box.innerHTML = "";
    if (!d.suggestions.length) { box.innerHTML = `<p class="empty-state" style="padding:6px">All pads outside zones fit the base thickness.</p>`; return; }
    for (const s of d.suggestions) {
        const row = document.createElement("div");
        row.className = "sug";
        const um = Math.round(s.thickness * 1000);
        row.innerHTML = `<div><strong>${escapeHtml(s.package)}</strong> → ${um} µm<div class="meta">${s.zones.length} zone(s), allowed ≤ ${Math.round(s.t_max * 1000)} µm</div></div>
            <button class="btn btn-xs btn-secondary">Preview</button>`;
        row.querySelector("button").addEventListener("click", async () => {
            setPackageHighlight(s.package, null);
            await openStepPanel(s.package, um);
            zoomToHighlight();
        });
        box.appendChild(row);
    }
}

// Gestrichelte Vorschau der vorgeschlagenen Zonen
function renderZoneProposals() {
    if (!state.zoneProposals?.length) return;
    ctx.save();
    ctx.setLineDash([6, 4]);
    ctx.lineWidth = 1.5;
    for (const z of state.zoneProposals) {
        const p0 = worldToScreen(z.x0, z.y1), p1 = worldToScreen(z.x1, z.y0);
        ctx.fillStyle = "rgba(55, 148, 255, 0.10)";
        ctx.strokeStyle = z.overlaps_zone ? "#d29922" : "#3794ff";
        ctx.fillRect(p0.x, p0.y, p1.x - p0.x, p1.y - p0.y);
        ctx.strokeRect(p0.x, p0.y, p1.x - p0.x, p1.y - p0.y);
    }
    ctx.restore();
}

function setPackageHighlight(pkg, pin = null) {
    // Packages ohne Pads auf der Platine (z. B. Bibliothek) nicht hervorheben
    if (pkg && !(state.apertures || []).some(a => a.pkg_name === pkg)) pkg = null;
    state.highlightPkg = pkg || null;
    state.highlightPin = pkg ? pin : null;
    const chip = document.getElementById("pkgHighlightChip");
    if (chip) {
        chip.classList.toggle("hidden", !pkg);
        if (pkg) {
            const n = (state.apertures || []).filter(a => a.pkg_name === pkg).length;
            document.getElementById("lblHighlightPkg").textContent = pkg;
            document.getElementById("lblHighlightCount").textContent = n;
            document.getElementById("lblHighlightPin").textContent = pin !== null && pin !== undefined ? ` · Pin ${pin}` : "";
        }
    }
    render();
}
window.setPackageHighlight = setPackageHighlight;

function fitToWorldBox(minX, minY, maxX, maxY, margin = 60) {
    const rect = container.getBoundingClientRect();
    const worldW = Math.max(maxX - minX, 1), worldH = Math.max(maxY - minY, 1);
    const availW = Math.max(10, rect.width - margin * 2), availH = Math.max(10, rect.height - margin * 2);
    state.zoom = Math.max(0.2, Math.min(150, Math.min(availW / worldW, availH / worldH)));
    state.panX = margin + (availW - worldW * state.zoom) / 2 - minX * state.zoom;
    state.panY = margin + (availH - worldH * state.zoom) / 2 - minY * state.zoom;
    render();
}

function zoomToHighlight() {
    const aps = (state.apertures || []).filter(a => a.pkg_name === state.highlightPkg && (state.highlightPin == null || a.pin_number === state.highlightPin));
    if (aps.length === 0) return;
    fitToWorldBox(Math.min(...aps.map(a => a.cx - a.w / 2)), Math.min(...aps.map(a => a.cy - a.h / 2)),
                  Math.max(...aps.map(a => a.cx + a.w / 2)), Math.max(...aps.map(a => a.cy + a.h / 2)));
}

function renderPackageHighlight() {
    if (!state.highlightPkg) return;
    for (const ap of state.apertures) {
        if (ap.pkg_name !== state.highlightPkg) continue;
        const isPin = state.highlightPin != null && ap.pin_number === state.highlightPin;
        const sp = worldToScreen(ap.cx - ap.w / 2, ap.cy + ap.h / 2);
        const sw = Math.max(3, ap.w * state.zoom), sh = Math.max(3, ap.h * state.zoom);
        ctx.strokeStyle = isPin ? "#fde047" : "#fb923c";
        ctx.lineWidth = isPin ? 3 : 2;
        ctx.strokeRect(sp.x - 2, sp.y - 2, sw + 4, sh + 4);
    }
}

function setupCanvas() {
    resizeCanvas();
    window.addEventListener("resize", () => {
        resizeCanvas();
        render();
    });
    // Größe/Zoom-Änderungen der Container zuverlässig erfassen (auch bei ausgeblendeter Ansicht)
    if (window.ResizeObserver) {
        new ResizeObserver(() => { resizeCanvas(); render(); }).observe(container);
        const pc = document.getElementById("pkgPreviewContainer");
        if (pc) new ResizeObserver(() => {
            if (typeof resizePkgCanvas === "function" && pkgInitialized) { resizePkgCanvas(); renderPkgPreview(); }
        }).observe(pc);
    }
}

function resizeCanvas() {
    const rect = container.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return; // ausgeblendet
    const dpr = window.devicePixelRatio || 1;
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.scale(dpr, dpr);
}

// -------------------------------------------------------------
// Data Fetching & Sync
// -------------------------------------------------------------
async function loadDataset(side) {
    try {
        const res = await fetch("/api/load_dataset", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ dataset: side })
        });
        const data = await res.json();
        await loadLayers();
        await refreshApertureData();
        fitToScreen();
    } catch (e) {
        console.error("Fehler beim Laden des Datensatzes:", e);
    }
}

async function refreshApertureData() {
    try {
        const res = await fetch("/api/apertures");
        const data = await res.json();
        state.bounds = data.bounds;
        state.apertures = data.apertures;
        // Eine Ansicht für TOP und BOT: andere Seite nur zum Zeichnen und Anklicken
        state.otherSide = data.other_side || null;
        state.otherApertures = data.other_apertures || [];
        state.otherComponents = data.other_components || [];
        state.pads = data.pads || [];
        state.components = data.components || [];
        state.zones = data.zones;
        state.zoneMargins = data.zone_margins || [1, 1];
        state.fiducials = data.fiducials;
        state.profile = data.profile;

        if (data.paste_type) {
            state.pasteType = data.paste_type;
            const ptSelect = document.getElementById("selectPasteType");
            if (ptSelect && ptSelect.value !== data.paste_type) {
                ptSelect.value = data.paste_type;
            }
        }
        if (data.min_bridge !== undefined) {
            state.minBridge = data.min_bridge;
            const lblMb = document.getElementById("lblMinBridge");
            if (lblMb) lblMb.innerText = data.min_bridge.toFixed(2);
        }
        if (data.bridge_violations) {
            state.bridgeViolations = data.bridge_violations;
            updateBridgeViolationsBadge();
        }

        await updateStats();
        render();

        if (state.selectedApertureId !== null) {
            selectAperture(state.selectedApertureId);
        }
    } catch (e) {
        console.error("Fehler bei Apertur-Sync:", e);
    }
}

// IPC-Klassenprofile: Grenzwerte je Klasse anzeigen und bearbeiten
const IPC_FIELDS = [
    ["name", "Name", "text"],
    ["min_ar", "Area Ratio ≥", "number", 0.01, "aperture area / wall area"],
    ["min_aspect", "Aspect Ratio ≥", "number", 0.05, "min. width / stencil thickness"],
    ["warn_margin_pct", "Warning band (%)", "number", 1, "below: critical"],
    ["balls_rect", "Grains rectangular", "number", 0.5, "x grain diameter across min. width"],
    ["balls_round", "Grains round", "number", 0.5, ""],
    ["min_bridge", "Min. web (mm)", "number", 0.01, "web width in stencil"],
    ["squeegee_clearance", "Step clearance squeegee (mm)", "number", 0.1, "in squeegee direction"],
    ["transverse_clearance", "Step clearance transverse (mm)", "number", 0.1, "across squeegee direction"],
];

function renderIpcProfileTable(data) {
    const t = document.getElementById("ipcProfileTable");
    const profiles = data.profiles || [];
    const defaults = Object.fromEntries((data.defaults || []).map(p => [p.class, p]));
    const act = (p) => p.class === data.active_class ? "active" : "";
    let html = `<tr><th></th>${profiles.map(p => `<th class="${act(p)}">Class ${p.class}${p.class === data.active_class ? " ✓" : ""}</th>`).join("")}</tr>`;
    for (const [key, label, type, step, hint] of IPC_FIELDS) {
        html += `<tr><td>${label}${hint ? `<span class="hint">${hint}</span>` : ""}</td>` + profiles.map(p => {
            const def = defaults[p.class]?.[key];
            const changed = type === "number" && def !== undefined && Math.abs(def - p[key]) > 1e-9;
            return `<td class="${act(p)}"><input type="${type}" data-class="${p.class}" data-key="${key}" ${type === "number" ? `step="${step}"` : ""}
                value="${escapeHtml(String(p[key]))}" title="${def !== undefined ? `Default: ${def}` : ""}" ${changed ? 'style="border-color:#f59e0b"' : ""}></td>`;
        }).join("") + "</tr>";
    }
    t.innerHTML = html;
}

async function openIpcProfiles() {
    const d = await (await fetch("/api/ipc/profiles")).json();
    renderIpcProfileTable(d);
    document.getElementById("lblIpcCustomer").innerText = d.customer || "Standard";
    document.getElementById("ipcProfileModal").classList.remove("hidden");
}

// Regel nur für einen Pin speichern ("Wie Package" = Pin-Anpassung entfernen)
async function savePinRule(target, ruleData) {
    noteRuleEdit();
    const cur = pkgState.ruleTarget;
    if (!cur || cur.kind !== "pin" || cur.id !== target.id || cur.pin !== target.pin) return;
    const url = `/api/package/${encodeURIComponent(target.id)}/pin/${target.pin}/override`;
    let res;
    if (ruleData.rule_type === "inherit") {
        res = await fetch(url, { method: "DELETE" });
    } else {
        // Größenvorgaben (B/H) der bestehenden Pin-Anpassung behalten
        const old = pinOverrideOf(target.pin) || {};
        const keep = {};
        for (const k of ["w", "h", "manual"]) if (old[k] !== undefined) keep[k] = old[k];
        res = await fetch(url, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ ...keep, ...ruleData }) });
    }
    const data = await res.json();
    const now = pkgState.ruleTarget;
    if (!now || now.kind !== "pin" || now.pin !== target.pin) return;
    if (data.preview) {
        pkgState.activePackageData = data.preview;
        populatePinDropdown(data.preview.pads, data.preview.pad_overrides);
        const dd = document.getElementById("selectPinDropdown");
        if (dd) dd.value = target.pin;
        const pad = data.preview.pads.find(p => p.pin === target.pin);
        if (pad) fillPinBarFromOverride(pinOverrideOf(target.pin), pad);
        if (ruleData.rule_type === "inherit") setPinRuleTarget(target.pin); else updateRuleSourceBanner();
        renderPkgPreview();
        renderPkgTree();
    }
    await refreshApertureData();
    refreshHistory();
}

async function saveIpcProfiles() {
    const byClass = {};
    document.querySelectorAll("#ipcProfileTable input[data-class]").forEach(inp => {
        const c = inp.dataset.class;
        byClass[c] = byClass[c] || {};
        byClass[c][inp.dataset.key] = inp.type === "number" ? parseFloat(inp.value) : inp.value;
    });
    for (const [c, vals] of Object.entries(byClass)) {
        await fetch(`/api/ipc/profile/${c}`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(vals) });
    }
    let msg = "IPC profiles applied";
    if (document.getElementById("chkIpcSaveLib").checked) {
        const cust = document.getElementById("lblIpcCustomer").innerText || "Standard";
        const r = await fetch("/api/library/save", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ customer: cust }) });
        msg += r.ok ? ` and saved to "${cust}"` : " (saving to library failed)";
    }
    document.getElementById("ipcProfileModal").classList.add("hidden");
    await refreshApertureData();
    await refreshHistory();
    showToast(msg);
}

function updateThicknessInput(force = false) {
    const el = document.getElementById("inputBaseThickness");
    const t = state.stats?.base_thickness;
    if (el && t && (force || document.activeElement !== el)) el.value = Math.round(t * 1000);
}

async function updateStats() {
    const res = await fetch("/api/stats");
    const data = await res.json();
    state.stats = data.stats;
    updateThicknessInput();

    document.getElementById("statTotal").innerText = state.stats.total_apertures.toLocaleString();
    document.getElementById("statOk").innerText = state.stats.ok_count.toLocaleString();
    document.getElementById("statWarn").innerText = state.stats.warn_count.toLocaleString();
    const wp = document.getElementById("warnPill");
    if (wp) wp.title = `Marginal (paste release unstable)
Vias in pad: ${state.stats.via_in_pad_count ?? 0}
Narrow apertures across squeegee: ${state.stats.squeegee_slot_count ?? 0}`;
    document.getElementById("statCrit").innerText = state.stats.crit_count.toLocaleString();
    document.getElementById("zonesCount").innerText = state.stats.zones_count;
    document.getElementById("lblMinAr").innerText = state.stats.ipc_min_ar.toFixed(2);
    const ipcSel = document.getElementById("ipcModeSelect");
    const mode = state.stats.ipc_class ? `class${state.stats.ipc_class}` : state.stats.ipc_mode;
    if (ipcSel && mode && [...ipcSel.options].some(o => o.value === mode) && document.activeElement !== ipcSel) ipcSel.value = mode;
    if (state.stats.min_bridge_width !== undefined) {
        const lblMb = document.getElementById("lblMinBridge");
        if (lblMb) lblMb.innerText = state.stats.min_bridge_width.toFixed(2);
    }

    if (state.stats.paste_type) {
        const ptEl = document.getElementById("selectPasteType");
        if (ptEl) ptEl.value = state.stats.paste_type;
    }
    if (state.stats.bridge_violations_count !== undefined) {
        updateBridgeViolationsBadge(state.stats.bridge_violations_count);
    }

    renderZonesList();
    renderCriticalList();
}

// -------------------------------------------------------------
// Coordinate Transforms (World mm <-> Screen px)
// -------------------------------------------------------------
// Viewport-Höhe pro Bild zwischenspeichern: getBoundingClientRect() erzwingt sonst bei jedem
// Punkt eine Layout-Berechnung (bei großen ODB++-Daten zehntausende Male pro Bild).
let viewHeightCache = null;
function viewHeight() {
    return viewHeightCache ?? container.getBoundingClientRect().height;
}

function worldToScreen(wx, wy) {
    const sx = state.panX + wx * state.zoom;
    const sy = viewHeight() - (state.panY + wy * state.zoom);
    return { x: sx, y: sy };
}

function screenToWorld(sx, sy) {
    const wx = (sx - state.panX) / state.zoom;
    const wy = (viewHeight() - sy - state.panY) / state.zoom;
    return { x: wx, y: wy };
}

// -------------------------------------------------------------
// Package Canvas Transforms & Measurement Tools
// -------------------------------------------------------------
// Editor zeigt das Package wie seine erste Instanz auf der Platine: gedreht und auf BOT gespiegelt
function pkgInstanceTf() {
    const bc = pkgState.boardContext;
    if (!bc || bc.refdes == null) return { c: 1, s: 0, m: 1 };
    const r = ((bc.rot || 0) * Math.PI) / 180;
    return { c: Math.cos(r), s: Math.sin(r), m: bc.mirror ? -1 : 1 };
}
// Package-Rahmen → Platinenlage (wie pkg_to_frame in rules.rs)
function pkgToInstance(x, y) {
    const t = pkgInstanceTf();
    return { x: t.m * (x * t.c + y * t.s), y: -x * t.s + y * t.c };
}
function instanceToPkg(x, y) {
    const t = pkgInstanceTf();
    const ux = x * t.m;
    return { x: ux * t.c - y * t.s, y: ux * t.s + y * t.c };
}

function pkgWorldToScreen(x, y) {
    ({ x, y } = pkgToInstance(x, y));
    const pCanvas = document.getElementById("pkgPreviewCanvas");
    if (!pCanvas) return { x: 0, y: 0 };
    const rect = pCanvas.getBoundingClientRect();
    const centerX = rect.width / 2 + pkgState.panX;
    const centerY = rect.height / 2 + pkgState.panY;
    const rotRad = ((pkgState.rotation || 0) * Math.PI) / 180;
    const c = Math.cos(rotRad), s = Math.sin(rotRad);
    // Entspricht ctx.translate(center); ctx.rotate(θ); ctx.scale(z, -z)
    return {
        x: centerX + pkgState.zoom * (x * c + y * s),
        y: centerY + pkgState.zoom * (x * s - y * c)
    };
}

// Bildschirm (clientX/Y) → Package-Koordinaten in mm (Umkehrung von pkgWorldToScreen)
function pkgScreenToWorld(clientX, clientY) {
    const pCanvas = document.getElementById("pkgPreviewCanvas");
    const rect = pCanvas.getBoundingClientRect();
    const vx = (clientX - rect.left - (rect.width / 2 + pkgState.panX)) / pkgState.zoom;
    const vy = (clientY - rect.top - (rect.height / 2 + pkgState.panY)) / pkgState.zoom;
    const rotRad = ((pkgState.rotation || 0) * Math.PI) / 180;
    const c = Math.cos(rotRad), s = Math.sin(rotRad);
    return instanceToPkg(vx * c + vy * s, vx * s - vy * c);
}

// Fangpunkte eines Elements: Zentrum, Ecken, Kantenmitten bzw. Polygon-Eckpunkte
function snapCandidates(cx, cy, w, h, pts, round) {
    const out = [{ x: cx, y: cy, kind: "Center" }];
    if (pts && pts.length >= 3 && pts.length <= 64) {
        for (const p of pts) out.push({ x: p[0], y: p[1], kind: "Corner" });
        for (let i = 0; i < pts.length; i++) {
            const a = pts[i], b = pts[(i + 1) % pts.length];
            out.push({ x: (a[0] + b[0]) / 2, y: (a[1] + b[1]) / 2, kind: "Edge" });
        }
    } else if (w && h) {
        const hw = w / 2, hh = h / 2;
        out.push({ x: cx - hw, y: cy, kind: "Edge" }, { x: cx + hw, y: cy, kind: "Edge" },
                 { x: cx, y: cy - hh, kind: "Edge" }, { x: cx, y: cy + hh, kind: "Edge" });
        if (!round) {
            out.push({ x: cx - hw, y: cy - hh, kind: "Corner" }, { x: cx + hw, y: cy - hh, kind: "Corner" },
                     { x: cx + hw, y: cy + hh, kind: "Corner" }, { x: cx - hw, y: cy + hh, kind: "Corner" });
        }
    }
    return out;
}

function nearestSnap(items, wx, wy, maxDist) {
    let best = null, bestD = maxDist;
    for (const it of items) {
        const reach = Math.max(it.w || 0, it.h || 0) / 2 + maxDist;
        if (Math.abs(it.cx - wx) > reach || Math.abs(it.cy - wy) > reach) continue;
        for (const c of snapCandidates(it.cx, it.cy, it.w, it.h, it.pts, it.round)) {
            const d = Math.hypot(c.x - wx, c.y - wy);
            if (d < bestD) {
                bestD = d;
                best = { x: c.x, y: c.y, label: `${it.label} · ${c.kind}` };
            }
        }
    }
    // Kein Fangpunkt in Reichweite: auf die nächste Kontur fangen (Lotfußpunkt)
    return best || nearestEdgeSnap(items, wx, wy, maxDist);
}

function nearestEdgeSnap(items, wx, wy, maxDist) {
    let best = null, bestD = maxDist;
    const seg = (ax, ay, bx, by, label) => {
        const dx = bx - ax, dy = by - ay, l2 = dx * dx + dy * dy;
        const t = l2 > 0 ? Math.max(0, Math.min(1, ((wx - ax) * dx + (wy - ay) * dy) / l2)) : 0;
        const x = ax + t * dx, y = ay + t * dy, d = Math.hypot(x - wx, y - wy);
        if (d < bestD) { bestD = d; best = { x, y, label: `${label} · Line` }; }
    };
    for (const it of items) {
        const reach = Math.max(it.w || 0, it.h || 0) / 2 + maxDist;
        if (!it.w && !it.pts) continue;
        if (Math.abs(it.cx - wx) > reach || Math.abs(it.cy - wy) > reach) continue;
        if (it.pts && it.pts.length >= 3) {
            for (let i = 0; i < it.pts.length; i++) {
                const a = it.pts[i], b = it.pts[(i + 1) % it.pts.length];
                seg(a[0], a[1], b[0], b[1], it.label);
            }
        } else if (it.round) {
            const r = it.w / 2, d0 = Math.hypot(wx - it.cx, wy - it.cy);
            if (d0 > 0 && Math.abs(d0 - r) < bestD) {
                bestD = Math.abs(d0 - r);
                best = { x: it.cx + (wx - it.cx) * r / d0, y: it.cy + (wy - it.cy) * r / d0, label: `${it.label} · Line` };
            }
        } else {
            const x0 = it.cx - it.w / 2, x1 = it.cx + it.w / 2, y0 = it.cy - it.h / 2, y1 = it.cy + it.h / 2;
            seg(x0, y0, x1, y0, it.label); seg(x1, y0, x1, y1, it.label); seg(x1, y1, x0, y1, it.label); seg(x0, y1, x0, y0, it.label);
        }
    }
    return best;
}

// Fangen im Board: Pad-/Slice-Ecken, Kantenmitten, Zentren und Bauteilzentren (8 px Fangradius).
function findSnapPointBoard(wx, wy) {
    const snapDistMm = 8 / state.zoom;
    const items = [];
    for (const ap of state.apertures || []) {
        const label = ap.comp_id ? `${ap.comp_id}` : "Pad";
        if (ap.slices) {
            for (const s of ap.slices) items.push({ cx: s.cx, cy: s.cy, w: s.w, h: s.h, pts: s.pts, round: s.shape === "circle", label });
        }
        items.push({ cx: ap.cx, cy: ap.cy, w: ap.w, h: ap.h, pts: ap.pts, round: ap.shape === "circle", label });
    }
    for (const c of state.components || []) {
        items.push({ cx: c.cx, cy: c.cy, w: 0, h: 0, label: c.refdes || "Component" });
    }
    return nearestSnap(items, wx, wy, snapDistMm);
}

// Fangen im Stencil Generator: Kupferpads und Pastenöffnungen (Ecken, Kantenmitten, Zentren).
function findSnapPointPkg(mmX, mmY) {
    const snapDistMm = 8 / pkgState.zoom;
    const data = pkgState.activePackageData;
    const items = [];
    for (const p of data?.pads || []) {
        items.push({ cx: p.cx, cy: p.cy, w: p.w, h: p.h, round: p.shape === "circle", label: `Pin ${p.pin}` });
    }
    for (const s of data?.paste || []) {
        items.push({ cx: s.cx, cy: s.cy, w: s.w, h: s.h, pts: s.pts, round: s.shape === "circle", label: s.pin ? `Paste ${s.pin}` : "Paste" });
    }
    return nearestSnap(items, mmX, mmY, snapDistMm);
}

// Setzt die Transformation auf CSS-Pixel (berücksichtigt devicePixelRatio, z. B. bei Seitenzoom).
function resetToCssPixels(targetCtx) {
    const dpr = window.devicePixelRatio || 1;
    targetCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
}

function drawCanvasScaleBar(targetCtx, canvasWidth, canvasHeight, zoom, offsetX = 14, offsetY = 65) {
    if (!zoom || zoom <= 0 || !targetCtx) return;

    const targetPx = 100;
    const ladder = [
        0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.25, 0.5,
        1.0, 2.0, 2.5, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0
    ];

    let chosenMm = ladder[0];
    let minDiff = Infinity;
    for (const step of ladder) {
        const px = step * zoom;
        const diff = Math.abs(px - targetPx);
        if (diff < minDiff && px >= 40 && px <= 240) {
            minDiff = diff;
            chosenMm = step;
        }
    }

    const barPx = chosenMm * zoom;
    const mils = (chosenMm * 39.3700787).toFixed(1);
    const mmStr = chosenMm >= 1 ? `${chosenMm} mm` : `${chosenMm} mm`;
    const label = `${mmStr} (${mils} mil)`;

    targetCtx.save();
    resetToCssPixels(targetCtx);

    const x = offsetX;
    const y = canvasHeight - offsetY;
    const boxW = Math.max(barPx + 20, 125);
    const boxH = 26;

    targetCtx.fillStyle = "rgba(15, 23, 42, 0.88)";
    targetCtx.strokeStyle = "rgba(255, 255, 255, 0.18)";
    targetCtx.lineWidth = 1;
    targetCtx.beginPath();
    targetCtx.roundRect(x - 6, y - 16, boxW, boxH, 6);
    targetCtx.fill();
    targetCtx.stroke();

    targetCtx.strokeStyle = "#38bdf8";
    targetCtx.lineWidth = 2.5;
    targetCtx.beginPath();
    targetCtx.moveTo(x, y + 4);
    targetCtx.lineTo(x + barPx, y + 4);
    targetCtx.moveTo(x, y - 1);
    targetCtx.lineTo(x, y + 7);
    targetCtx.moveTo(x + barPx, y - 1);
    targetCtx.lineTo(x + barPx, y + 7);
    if (barPx > 60) {
        targetCtx.moveTo(x + barPx / 2, y + 1);
        targetCtx.lineTo(x + barPx / 2, y + 6);
    }
    targetCtx.stroke();

    targetCtx.fillStyle = "#ffffff";
    targetCtx.font = "600 10px Inter, -apple-system, BlinkMacSystemFont, sans-serif";
    targetCtx.textAlign = "center";
    targetCtx.textBaseline = "middle";
    targetCtx.fillText(label, x + barPx / 2, y - 5);

    targetCtx.restore();
}

function drawMeasurementLayer(targetCtx, mode, toScreenFn, zoom) {
    if (!targetCtx) return;

    for (const m of measureState.measurements) {
        if (m.mode !== mode) continue;
        drawSingleMeasurement(targetCtx, m.start, m.end, toScreenFn, false);
    }

    if (measureState.active && currentWorkspaceMode === mode) {
        if (measureState.startPoint && measureState.currentPoint) {
            drawSingleMeasurement(targetCtx, measureState.startPoint, measureState.currentPoint, toScreenFn, true);
        }

        if (measureState.hoverSnap) {
            const s = toScreenFn(measureState.hoverSnap.x, measureState.hoverSnap.y);
            targetCtx.save();
            resetToCssPixels(targetCtx);
            targetCtx.strokeStyle = "#38bdf8";
            targetCtx.fillStyle = "rgba(56, 189, 248, 0.4)";
            targetCtx.lineWidth = 1.5;
            const sz = 6;
            targetCtx.beginPath();
            targetCtx.moveTo(s.x, s.y - sz);
            targetCtx.lineTo(s.x + sz, s.y);
            targetCtx.lineTo(s.x, s.y + sz);
            targetCtx.lineTo(s.x - sz, s.y);
            targetCtx.closePath();
            targetCtx.fill();
            targetCtx.stroke();

            if (measureState.hoverSnap.label) {
                targetCtx.fillStyle = "#38bdf8";
                targetCtx.font = "bold 9px Inter, sans-serif";
                targetCtx.textAlign = "left";
                targetCtx.textBaseline = "middle";
                targetCtx.fillText(measureState.hoverSnap.label, s.x + 8, s.y);
            }
            targetCtx.restore();
        }
    }
}

function drawSingleMeasurement(targetCtx, pt1, pt2, toScreenFn, isActive) {
    const s1 = toScreenFn(pt1.x, pt1.y);
    const s2 = toScreenFn(pt2.x, pt2.y);
    const dx = pt2.x - pt1.x;
    const dy = pt2.y - pt1.y;
    const dist = Math.hypot(dx, dy);
    if (dist < 0.0001) return;

    targetCtx.save();
    resetToCssPixels(targetCtx);

    const color = isActive ? "#38bdf8" : "#0ea5e9";
    targetCtx.strokeStyle = color;
    targetCtx.lineWidth = isActive ? 1.8 : 1.5;

    targetCtx.beginPath();
    targetCtx.moveTo(s1.x, s1.y);
    targetCtx.lineTo(s2.x, s2.y);
    targetCtx.stroke();

    const tickSz = 5;
    [s1, s2].forEach(s => {
        targetCtx.beginPath();
        targetCtx.moveTo(s.x - tickSz, s.y);
        targetCtx.lineTo(s.x + tickSz, s.y);
        targetCtx.moveTo(s.x, s.y - tickSz);
        targetCtx.lineTo(s.x, s.y + tickSz);
        targetCtx.stroke();
    });

    if (Math.abs(dx) > 0.05 && Math.abs(dy) > 0.05) {
        const sCorner = toScreenFn(pt2.x, pt1.y);
        targetCtx.strokeStyle = "rgba(56, 189, 248, 0.35)";
        targetCtx.setLineDash([3, 3]);
        targetCtx.beginPath();
        targetCtx.moveTo(s1.x, s1.y);
        targetCtx.lineTo(sCorner.x, sCorner.y);
        targetCtx.lineTo(s2.x, s2.y);
        targetCtx.stroke();
        targetCtx.setLineDash([]);
    }

    const midX = (s1.x + s2.x) / 2;
    const midY = (s1.y + s2.y) / 2;

    const mils = (dist * 39.3700787).toFixed(1);
    const mmStr = dist >= 10 ? dist.toFixed(2) : dist.toFixed(3);
    const dxStr = Math.abs(dx).toFixed(3);
    const dyStr = Math.abs(dy).toFixed(3);

    const line1 = `${mmStr} mm (${mils} mil)`;
    const line2 = `ΔX: ${dxStr} | ΔY: ${dyStr}`;

    targetCtx.font = "bold 11px Inter, sans-serif";
    const textW1 = targetCtx.measureText(line1).width;
    targetCtx.font = "9px Inter, sans-serif";
    const textW2 = targetCtx.measureText(line2).width;
    const padW = Math.max(textW1, textW2) + 16;
    const padH = 34;

    // Beschriftung neben die Messlinie (Normale nach oben), damit die gemessene Stelle sichtbar bleibt
    const ls = Math.hypot(s2.x - s1.x, s2.y - s1.y) || 1;
    let nx = -(s2.y - s1.y) / ls, ny = (s2.x - s1.x) / ls;
    if (ny > 0 || (ny === 0 && nx < 0)) { nx = -nx; ny = -ny; }
    const off = Math.abs(nx) * padW / 2 + Math.abs(ny) * padH / 2 + 10;
    const lx = midX + nx * off, ly = midY + ny * off;

    targetCtx.fillStyle = "rgba(15, 23, 42, 0.90)";
    targetCtx.strokeStyle = color;
    targetCtx.lineWidth = 1;
    targetCtx.beginPath();
    targetCtx.roundRect(lx - padW / 2, ly - padH / 2, padW, padH, 5);
    targetCtx.fill();
    targetCtx.stroke();

    targetCtx.fillStyle = "#ffffff";
    targetCtx.font = "bold 11px Inter, sans-serif";
    targetCtx.textAlign = "center";
    targetCtx.textBaseline = "middle";
    targetCtx.fillText(line1, lx, ly - 6);

    targetCtx.fillStyle = "#94a3b8";
    targetCtx.font = "9px Inter, sans-serif";
    targetCtx.fillText(line2, lx, ly + 7);

    targetCtx.restore();
}

function toggleMeasureTool(forceState = null) {
    measureState.active = forceState !== null ? forceState : !measureState.active;
    measureState.startPoint = null;
    measureState.currentPoint = null;
    measureState.hoverSnap = null;

    const btnBoard = document.getElementById("btnMeasureBoard");
    const btnPkg = document.getElementById("btnMeasurePkg");
    const bannerBoard = document.getElementById("measureBanner");
    const bannerPkg = document.getElementById("measureBannerPkg");
    const boardContainer = document.getElementById("canvasContainer");
    const pkgContainerEl = document.getElementById("pkgPreviewContainer");

    if (measureState.active) {
        btnBoard?.classList.add("active");
        btnPkg?.classList.add("active");
        bannerBoard?.classList.remove("hidden");
        bannerPkg?.classList.remove("hidden");
        boardContainer?.classList.add("measuring-mode");
        pkgContainerEl?.classList.add("measuring-mode");
        showToast("Measure tool active: click 1st point, then 2nd point");
    } else {
        btnBoard?.classList.remove("active");
        btnPkg?.classList.remove("active");
        bannerBoard?.classList.add("hidden");
        bannerPkg?.classList.add("hidden");
        boardContainer?.classList.remove("measuring-mode");
        pkgContainerEl?.classList.remove("measuring-mode");
    }

    render();
    renderPkgPreview();
}

function updateMeasureButtonsVisibility() {
    const hasMeasurements = measureState.measurements && measureState.measurements.length > 0;
    const btnBoard = document.getElementById("btnClearMeasureBoard");
    const btnPkg = document.getElementById("btnClearMeasurePkg");
    if (btnBoard) {
        if (hasMeasurements) btnBoard.classList.remove("hidden");
        else btnBoard.classList.add("hidden");
    }
    if (btnPkg) {
        if (hasMeasurements) btnPkg.classList.remove("hidden");
        else btnPkg.classList.add("hidden");
    }
}

function clearMeasurements() {
    measureState.measurements = [];
    measureState.startPoint = null;
    measureState.currentPoint = null;
    updateMeasureButtonsVisibility();
    showToast("Measurements cleared");
    render();
    renderPkgPreview();
}

function fitToScreen() {
    const rect = container.getBoundingClientRect();
    let [minX, minY, maxX, maxY] = state.bounds || [0, 0, 0, 0];
    // Auf die Pastenöffnungen einpassen: Zeichnungslagen (Bemaßung, Schriftfeld) machen die Gesamtgrenzen oft viel größer
    if (state.apertures && state.apertures.length > 0) {
        minX = Math.min(...state.apertures.map(a => a.cx - a.w / 2));
        maxX = Math.max(...state.apertures.map(a => a.cx + a.w / 2));
        minY = Math.min(...state.apertures.map(a => a.cy - a.h / 2));
        maxY = Math.max(...state.apertures.map(a => a.cy + a.h / 2));
    }
    const worldW = (maxX - minX) || 200;
    const worldH = (maxY - minY) || 300;

    const margin = 40;
    const availW = Math.max(10, rect.width - margin * 2);
    const availH = Math.max(10, rect.height - margin * 2);

    const scaleX = availW / worldW;
    const scaleY = availH / worldH;
    state.zoom = Math.min(scaleX, scaleY);

    state.panX = margin + (availW - worldW * state.zoom) / 2 - minX * state.zoom;
    state.panY = margin + (availH - worldH * state.zoom) / 2 - minY * state.zoom;

    render();
}

// -------------------------------------------------------------
// High-Performance Rendering Loop (Canvas 2D)
// -------------------------------------------------------------
// Mehrere render()-Aufrufe (Mausbewegung, Zoom, Datenänderung) zu einem Bild je Frame bündeln
let renderQueued = false;
function render() {
    if (renderQueued) return;
    renderQueued = true;
    requestAnimationFrame(() => {
        renderQueued = false;
        renderNow();
    });
}

function renderNow() {
    const rect = container.getBoundingClientRect();
    viewHeightCache = rect.height;
    try {
        renderFrame(rect);
    } finally {
        viewHeightCache = null;
    }
}

function renderFrame(rect) {
    ctx.clearRect(0, 0, rect.width, rect.height);

    // 1. Grid / Background
    renderBackgroundGrid(rect);

    // Empty state if nothing loaded
    const hasData = (state.apertures && state.apertures.length > 0) || (state.layers && state.layers.length > 0);
    if (!hasData) {
        ctx.save();
        ctx.fillStyle = "rgba(148, 163, 184, 0.75)";
        ctx.font = "bold 18px Inter, sans-serif";
        ctx.textAlign = "center";
        ctx.fillText("SMT StencilCraft ready", rect.width / 2, rect.height / 2 - 15);

        ctx.fillStyle = "rgba(100, 116, 139, 0.9)";
        ctx.font = "14px Inter, sans-serif";
        ctx.fillText("Click 'Open ODB++ / Gerber' above or drop a file / ZIP here.", rect.width / 2, rect.height / 2 + 18);
        ctx.restore();
        updateCoordsDisplay();
        return;
    }

    // 2. Custom & Imported Multi-Layers (Pads, Copper, Mask, Outline, Silk, etc.)
    renderCustomLayers();

    // 3. Fallback Profile (Kontur) if not in layerPrimitives
    renderProfile();

    // 4. Components (Bauteil-Layer / RefDes)
    renderComponents();

    // 5. Copper Pads (Grün) fallback if not in layerPrimitives
    renderPads();

    // 6. Zones (Stufen)
    renderZones();

    // 7. Fiducials
    renderFiducials();

    // 8. Apertures (Paste / Pink oder Heatmap)
    renderApertures();

    // 8b. Bauteil-Beschriftung über Kupfer und Paste
    renderComponentLabels();

    // 9. Selected Aperture Highlight
    if (state.selectedAperture) {
        renderSelectionHighlight();
    }
    renderMultiSelection();
    renderFocusComponent();

    // 9b. DFM Bridge Violations Overlay (Kollisionswarnung für Laserschnitt)
    renderBridgeViolations();

    // 10. Zone Drawing Preview
    if (state.isDrawingZone && state.drawStartWorld && state.drawCurrentWorld) {
        renderZoneDrawingPreview();
    }

    // 11. Measurements (Board View)
    drawMeasurementLayer(ctx, "board", worldToScreen, state.zoom);

    // 12. Dynamic CAD Scale Bar (Maßstab)
    drawCanvasScaleBar(ctx, rect.width, rect.height, state.zoom, 14, 65);

    updateCoordsDisplay();
}

function renderBackgroundGrid(rect) {
    ctx.strokeStyle = "#161b26";
    ctx.lineWidth = 1;
    const stepWorld = state.zoom > 10 ? 1 : (state.zoom > 3 ? 5 : 10);
    const p0 = screenToWorld(0, rect.height);
    const p1 = screenToWorld(rect.width, 0);

    const startX = Math.floor(p0.x / stepWorld) * stepWorld;
    const endX = Math.ceil(p1.x / stepWorld) * stepWorld;
    const startY = Math.floor(p0.y / stepWorld) * stepWorld;
    const endY = Math.ceil(p1.y / stepWorld) * stepWorld;

    ctx.beginPath();
    for (let x = startX; x <= endX; x += stepWorld) {
        const s = worldToScreen(x, 0);
        ctx.moveTo(s.x, 0);
        ctx.lineTo(s.x, rect.height);
    }
    for (let y = startY; y <= endY; y += stepWorld) {
        const s = worldToScreen(0, y);
        ctx.moveTo(0, s.y);
        ctx.lineTo(rect.width, s.y);
    }
    ctx.stroke();
}

// Lagen-Primitive einmalig zu Path2D (Weltkoordinaten) bündeln: eine Füllung + je Linienbreite ein Pfad
const layerPathCache = new WeakMap();
function layerPaths(prims) {
    let c = layerPathCache.get(prims);
    if (c) return c;
    const fill = new Path2D();
    const lines = new Map();
    for (const p of prims) {
        if (p.t === 'r') {
            fill.rect(p.x - p.w / 2, p.y - p.h / 2, p.w, p.h);
        } else if (p.t === 'c') {
            fill.moveTo(p.x + p.r, p.y);
            fill.arc(p.x, p.y, p.r, 0, Math.PI * 2);
        } else if (p.t === 'l') {
            const w = Math.round((p.w || 0.1) * 1000) / 1000;
            let path = lines.get(w);
            if (!path) { path = new Path2D(); lines.set(w, path); }
            path.moveTo(p.x1, p.y1);
            path.lineTo(p.x2, p.y2);
        } else if (p.t === 'p' && p.pts && p.pts.length >= 3) {
            // Einheitlicher Umlaufsinn (wie rect/arc), sonst löschen sich überlappende Flächen gegenseitig aus
            let a2 = 0;
            for (let i = 0, n = p.pts.length; i < n; i++) {
                const q = p.pts[i], r = p.pts[(i + 1) % n];
                a2 += q[0] * r[1] - r[0] * q[1];
            }
            const pts = a2 < 0 ? [...p.pts].reverse() : p.pts;
            fill.moveTo(pts[0][0], pts[0][1]);
            for (let i = 1; i < pts.length; i++) fill.lineTo(pts[i][0], pts[i][1]);
            fill.closePath();
        }
    }
    c = { fill, lines };
    layerPathCache.set(prims, c);
    return c;
}

function renderCustomLayers() {
    if (!state.layers || state.layers.length === 0) return;
    const rect = container.getBoundingClientRect();

    // Layer types render order: outline -> copper -> mask -> zone -> component -> other -> fiducial -> paste_orig
    const typeOrder = { 'profile': 0, 'copper': 1, 'mask': 2, 'zone': 3, 'component': 4, 'other': 5, 'fiducial': 6, 'paste_orig': 7, 'paste': 8 };
    const sorted = [...state.layers].sort((a, b) => (typeOrder[a.type] ?? 5) - (typeOrder[b.type] ?? 5));

    for (const layer of sorted) {
        if (layer.visible === false) continue;
        // The paste layer apertures are rendered with interactive IPC/slicing in renderApertures()
        if (layer.type === 'paste') continue;

        const data = state.layerPrimitives[layer.id];
        if (!data || !data.prims || data.prims.length === 0) continue;
        const paths = layerPaths(data.prims);

        ctx.save();
        ctx.globalAlpha = (layer.opacity || 0.85) * state.globalLayerOpacity;
        ctx.fillStyle = layer.color || "#10b981";
        ctx.strokeStyle = layer.color || "#10b981";
        ctx.lineCap = "round";
        // Weltkoordinaten → Bildschirm als eine Transformation; die Pfade werden nicht neu aufgebaut
        const dpr = window.devicePixelRatio || 1;
        const z = state.zoom;
        ctx.setTransform(dpr * z, 0, 0, -dpr * z, dpr * state.panX, dpr * (rect.height - state.panY));
        ctx.fill(paths.fill);
        for (const [w, path] of paths.lines) {
            ctx.lineWidth = Math.max(w, 1 / z);
            ctx.stroke(path);
        }
        ctx.restore();
    }
}

function renderProfile() {
    const profLayer = state.layers ? state.layers.find(l => l.type === 'profile') : null;
    if (profLayer && !profLayer.visible) return;
    if (!state.showProfile) return;

    // If profile is already drawn via layerPrimitives, skip fallback
    if (profLayer && state.layerPrimitives[profLayer.id]?.prims?.length > 0) return;

    ctx.strokeStyle = "#475569";
    ctx.lineWidth = Math.max(1, 0.2 * state.zoom);
    ctx.beginPath();
    for (const seg of state.profile) {
        const s0 = worldToScreen(seg[0][0], seg[0][1]);
        const s1 = worldToScreen(seg[1][0], seg[1][1]);
        ctx.moveTo(s0.x, s0.y);
        ctx.lineTo(s1.x, s1.y);
    }
    ctx.stroke();
}

function renderPads() {
    const hasCopperInLayers = state.layers && state.layers.some(l => (l.type === 'copper' || l.type === 'mask') && state.layerPrimitives[l.id]?.prims?.length > 0);
    if (hasCopperInLayers) return;

    const padLayer = state.layers ? state.layers.find(l => l.type === 'copper' || l.type === 'mask') : null;
    if (padLayer && !padLayer.visible) return;
    if (!state.showPads) return;

    // Kupfer-Pads in grün rendern (wie im Kunden-Screenshot)
    ctx.fillStyle = "rgba(16, 185, 129, 0.45)";
    ctx.strokeStyle = "#10b981";
    ctx.lineWidth = Math.max(1, 0.15 * state.zoom);

    for (const p of state.pads) {
        const sp = worldToScreen(p.x0, p.y1);
        const sw = Math.max(1, p.w * state.zoom);
        const sh = Math.max(1, p.h * state.zoom);
        ctx.fillRect(sp.x, sp.y, sw, sh);
        ctx.strokeRect(sp.x, sp.y, sw, sh);
    }
}

// Sichtbare Bauteile je Seite: aktive Seite aus state.components, andere Seite aus den Lagen-Primitiven
function visibleComponentSets() {
    if (!state.showComponents) return [];
    const compLayers = (state.layers || []).filter(l => l.type === 'component');
    const own = compLayers.find(l => (l.side || "top") === (state.side || "top"));
    const other = compLayers.filter(l => l !== own && l.visible !== false);
    const sets = [{ dim: true, list: other.flatMap(l => (state.layerPrimitives[l.id]?.prims || []).filter(p => p.t === 'comp')
        .map(p => ({ cx: p.x, cy: p.y, width: p.w, height: p.h, rot: p.rot, refdes: p.refdes, outline: p.o, pin1: p.p1, pins: p.pn }))) }];
    if (!own || own.visible !== false) sets.push({ dim: false, list: state.components });
    return sets;
}

// Körper wie PCB-Investigator: gefüllter Umriss unter Kupfer/Paste
function renderComponents() {
    for (const { dim, list } of visibleComponentSets()) {
        ctx.strokeStyle = dim ? "rgba(214, 170, 90, 0.35)" : "rgba(214, 170, 90, 0.85)";
        ctx.fillStyle = dim ? "rgba(110, 85, 45, 0.18)" : "rgba(110, 85, 45, 0.40)";
        ctx.lineWidth = 1;
        for (const c of list) {
            ctx.beginPath();
            if (c.outline && c.outline.length >= 3) {
                c.outline.forEach(([x, y], i) => { const q = worldToScreen(x, y); i ? ctx.lineTo(q.x, q.y) : ctx.moveTo(q.x, q.y); });
            } else {
                // Ohne CAD-Umriss: gedrehtes Rechteck aus der Package-Größe
                const a = -(c.rot || 0) * Math.PI / 180, ca = Math.cos(a), sa = Math.sin(a), q0 = worldToScreen(c.cx, c.cy);
                const hw = c.width * state.zoom / 2, hh = c.height * state.zoom / 2;
                [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].forEach(([x, y], i) => ctx[i ? "lineTo" : "moveTo"](q0.x + x * ca - y * sa, q0.y + x * sa + y * ca));
            }
            ctx.closePath();
            ctx.fill();
            ctx.stroke();
        }
    }
}

// Beschriftung über allen Lagen: RefDes mittig in Körpergröße, Pin-Namen auf den Pads, Pin 1 markiert
function renderComponentLabels() {
    const z = state.zoom;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    for (const { dim, list } of visibleComponentSets()) {
        for (const c of list) {
            let bw = c.width, bh = c.height, mx = c.cx, my = c.cy;
            if (c.outline && c.outline.length >= 3) {
                const xs = c.outline.map(p => p[0]), ys = c.outline.map(p => p[1]);
                const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
                [bw, bh, mx, my] = [x1 - x0, y1 - y0, (x0 + x1) / 2, (y0 + y1) / 2];
            }
            const q = worldToScreen(mx, my);
            if (q.x < -200 || q.y < -200 || q.x > canvas.width + 200 || q.y > canvas.height + 200) continue;
            if (c.refdes) {
                const px = Math.min(bh * z * 0.5, bw * z / (c.refdes.length * 0.62), 64);
                if (px >= 7) {
                    ctx.font = `${Math.round(px)}px Inter, sans-serif`;
                    ctx.fillStyle = dim ? "rgba(249, 168, 212, 0.45)" : "rgba(249, 168, 212, 0.9)";
                    ctx.fillText(c.refdes, q.x, q.y);
                }
            }
            let named = false;
            for (const [name, x, y, sz] of c.pins || []) {
                const px = Math.min(sz * z * 0.55, 14);
                if (px < 6) continue;
                const p = worldToScreen(x, y);
                ctx.font = `${Math.round(px)}px Inter, sans-serif`;
                // Pin 1 grün hervorheben (wie PCB-Investigator)
                const first = c.pin1 && Math.abs(x - c.pin1[0]) < 1e-3 && Math.abs(y - c.pin1[1]) < 1e-3;
                ctx.fillStyle = first ? "#84cc16" : dim ? "rgba(255, 255, 255, 0.4)" : "rgba(255, 255, 255, 0.85)";
                ctx.fillText(name, p.x, p.y);
                named = named || first;
            }
            if (c.pin1 && z > 1.5 && !named) {
                const p = worldToScreen(c.pin1[0], c.pin1[1]);
                ctx.fillStyle = "#84cc16";
                ctx.beginPath();
                ctx.arc(p.x, p.y, Math.max(1.5, Math.min(4, 0.08 * z)), 0, Math.PI * 2);
                ctx.fill();
            }
        }
    }
    ctx.textAlign = "start";
    ctx.textBaseline = "alphabetic";
}

function renderZones() {
    const zoneLayer = state.layers ? state.layers.find(l => l.type === 'zone') : null;
    if (zoneLayer && !zoneLayer.visible) return;
    if (!state.showZones) return;

    for (const z of state.zones) {
        const p0 = worldToScreen(z.x0, z.y1);
        const p1 = worldToScreen(z.x1, z.y0);
        const sw = Math.abs(p1.x - p0.x);
        const sh = Math.abs(p1.y - p0.y);

        ctx.fillStyle = z.is_step_down ? "rgba(192, 132, 252, 0.18)" : "rgba(244, 63, 94, 0.18)";
        ctx.fillRect(p0.x, p0.y, sw, sh);

        ctx.strokeStyle = z.is_step_down ? "#c084fc" : "#f43f5e";
        ctx.lineWidth = Math.max(1.5, 0.3 * state.zoom);
        ctx.setLineDash([4, 4]);
        ctx.strokeRect(p0.x, p0.y, sw, sh);
        ctx.setLineDash([]);

        ctx.font = "bold 11px Inter, sans-serif";
        ctx.fillStyle = z.is_step_down ? "#e9d5ff" : "#fecdd3";
        const label = `${z.name} (${Math.round(z.thickness * 1000)} µm)`;
        ctx.fillText(label, p0.x + 6, p0.y + 16);
        if (z.id === state.selectedZoneId) {
            ctx.strokeStyle = "#3794ff";
            ctx.lineWidth = 2;
            ctx.strokeRect(p0.x, p0.y, sw, sh);
            ctx.fillStyle = "#3794ff";
            for (const h of zoneHandles(z)) ctx.fillRect(h.x - 4, h.y - 4, 8, 8);
        }
    }
    renderZoneProposals();
}

// Griffe einer Zone in Bildschirmkoordinaten (Ecken und Kantenmitten)
function zoneHandles(z) {
    const a = worldToScreen(z.x0, z.y1), b = worldToScreen(z.x1, z.y0);
    const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2;
    return [
        { k: "nw", x: a.x, y: a.y }, { k: "n", x: mx, y: a.y }, { k: "ne", x: b.x, y: a.y }, { k: "e", x: b.x, y: my },
        { k: "se", x: b.x, y: b.y }, { k: "s", x: mx, y: b.y }, { k: "sw", x: a.x, y: b.y }, { k: "w", x: a.x, y: my },
    ];
}

function selectZone(id, zoom = true) {
    state.selectedZoneId = id;
    const z = (state.zones || []).find(x => x.id === id);
    if (z && zoom) {
        const m = Math.max(z.x1 - z.x0, z.y1 - z.y0) * 0.25 + 1;
        fitToWorldBox(z.x0 - m, z.y0 - m, z.x1 + m, z.y1 + m);
    }
    renderZonesList();
    render();
}

async function saveZoneGeometry(z) {
    await fetch(`/api/zone/${z.id}`, { method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ x0: z.x0, y0: z.y0, x1: z.x1, y1: z.y1, thickness: z.thickness }) });
    await refreshApertureData();
    refreshHistory();
}

function renderFiducials() {
    const fidLayer = state.layers ? state.layers.find(l => l.type === 'fiducial') : null;
    if (fidLayer && !fidLayer.visible) return;
    if (!state.showFiducials) return;

    if (fidLayer && state.layerPrimitives[fidLayer.id]?.prims?.length > 0) return;

    ctx.fillStyle = "#f43f5e";
    for (const f of state.fiducials) {
        const s = worldToScreen(f[0], f[1]);
        const sr = Math.max(3, f[2] * state.zoom);
        ctx.beginPath();
        ctx.arc(s.x, s.y, sr, 0, Math.PI * 2);
        ctx.fill();
    }
}

// Eine Öffnung (Slices, Kreis, Rechteck, Polygon) mit dem aktuellen fillStyle füllen
function fillApertureShape(ap) {
    const poly = pts => {
        ctx.beginPath();
        for (let i = 0; i < pts.length; i++) {
            const sp = worldToScreen(pts[i][0], pts[i][1]);
            if (i === 0) ctx.moveTo(sp.x, sp.y);
            else ctx.lineTo(sp.x, sp.y);
        }
        ctx.closePath();
        ctx.fill();
    };
    const rect = (cx, cy, w, h) => {
        const sp = worldToScreen(cx - w / 2, cy + h / 2);
        ctx.fillRect(sp.x, sp.y, Math.max(1, w * state.zoom), Math.max(1, h * state.zoom));
    };
    // Slices (leere Liste = keine Öffnung, z. B. Ignore)
    if (ap.slices) {
        for (const sl of ap.slices) {
            if (sl.pts && sl.pts.length > 0) poly(sl.pts);
            else rect(sl.cx, sl.cy, sl.w, sl.h);
        }
        return;
    }
    if (ap.shape === "circle") {
        const sp = worldToScreen(ap.cx, ap.cy);
        ctx.beginPath();
        ctx.arc(sp.x, sp.y, Math.max(1, (ap.w / 2) * state.zoom), 0, Math.PI * 2);
        ctx.fill();
    } else if (ap.shape === "polygon" && ap.pts) {
        poly(ap.pts);
    } else {
        rect(ap.cx, ap.cy, ap.w, ap.h);
    }
}

const stencilLayerOf = side => (state.layers || []).find(l => l.id === `stencil_mod_${side}`);

function renderApertures() {
    // Andere Seite (z. B. bot_stencil) darunter, wenn ihre Lage eingeblendet ist
    const other = state.otherSide && stencilLayerOf(state.otherSide);
    if (other?.visible && state.otherApertures?.length) {
        ctx.globalAlpha = other.opacity ?? 0.85;
        ctx.fillStyle = other.color || "#94a3b8";
        for (const ap of state.otherApertures) fillApertureShape(ap);
        ctx.globalAlpha = 1.0;
    }

    const own = stencilLayerOf(state.side);
    const pasteLayers = state.layers ? state.layers.filter(l => l.type === 'paste') : [];
    if (own) {
        if (!own.visible) { renderPackageHighlight(); return; }
    } else if (pasteLayers.length > 0) {
        if (!pasteLayers.some(l => l.visible)) return;
    } else if (!state.showPaste) {
        return;
    }

    const isHeatmap = state.colorMode === "ipc";
    const isLayers = state.colorMode === "layers";
    const activePasteLayer = own || pasteLayers.find(l => l.visible);
    const pasteColor = (activePasteLayer && activePasteLayer.color) ? activePasteLayer.color : "rgba(244, 114, 182, 0.85)";

    const dimOthers = state.highlightPkg && state.highlightDim !== false;
    for (const ap of state.apertures) {
        let fillColor = "#38bdf8";
        ctx.globalAlpha = dimOthers && ap.pkg_name !== state.highlightPkg ? 0.22 : 1.0;

        if (isHeatmap) {
            if (ap.status === "OK") fillColor = "#10b981";
            else if (ap.status === "WARN") fillColor = "#f59e0b";
            else fillColor = "#ef4444";
        } else if (isLayers) {
            fillColor = pasteColor;
        } else if (state.colorMode === "thickness") {
            fillColor = ap.thick < 0.120 ? "#c084fc" : (ap.thick > 0.120 ? "#f43f5e" : "#38bdf8");
        }
        ctx.fillStyle = fillColor;
        fillApertureShape(ap);
    }
    ctx.globalAlpha = 1.0;
    renderPackageHighlight();
}

function renderSelectionHighlight() {
    const sel = state.selectedAperture;
    if (!sel) return;

    if (state.highlightIdentical) {
        ctx.strokeStyle = "rgba(250, 204, 21, 0.4)";
        ctx.lineWidth = Math.max(1.5, 0.15 * state.zoom);
        for (const ap of state.apertures) {
            if (ap.pkg === sel.package_group && ap.id !== sel.id) {
                const sp = worldToScreen(ap.cx - ap.w/2, ap.cy + ap.h/2);
                const sw = Math.max(1, ap.w * state.zoom);
                const sh = Math.max(1, ap.h * state.zoom);
                ctx.strokeRect(sp.x - 1, sp.y - 1, sw + 2, sh + 2);
            }
        }
    }

    const sp = worldToScreen(sel.cx - sel.width/2, sel.cy + sel.height/2);
    const sw = Math.max(2, sel.width * state.zoom);
    const sh = Math.max(2, sel.height * state.zoom);

    ctx.strokeStyle = "#facc15";
    ctx.lineWidth = 2.5;
    ctx.strokeRect(sp.x - 2, sp.y - 2, sw + 4, sh + 4);

    const center = worldToScreen(sel.cx, sel.cy);
    ctx.strokeStyle = "rgba(250, 204, 21, 0.8)";
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 3]);
    ctx.beginPath();
    ctx.moveTo(center.x - 15, center.y);
    ctx.lineTo(center.x + 15, center.y);
    ctx.moveTo(center.x, center.y - 15);
    ctx.lineTo(center.x, center.y + 15);
    ctx.stroke();
    ctx.setLineDash([]);
}

function renderBridgeViolations() {
    if (!state.bridgeViolations || state.bridgeViolations.length === 0) return;

    ctx.save();
    for (const v of state.bridgeViolations) {
        if (!v.p1 || !v.p2) continue;
        const p1 = worldToScreen(v.p1[0], v.p1[1]);
        const p2 = worldToScreen(v.p2[0], v.p2[1]);

        const isCrit = v.severity === "CRIT";
        const strokeColor = isCrit ? "#ef4444" : "#f59e0b";
        const glowColor = isCrit ? "rgba(239, 68, 68, 0.35)" : "rgba(245, 158, 11, 0.35)";

        // Warning Glow line
        ctx.strokeStyle = glowColor;
        ctx.lineWidth = 6;
        ctx.beginPath();
        ctx.moveTo(p1.x, p1.y);
        ctx.lineTo(p2.x, p2.y);
        ctx.stroke();

        // Dashed line
        ctx.strokeStyle = strokeColor;
        ctx.lineWidth = 2.0;
        ctx.setLineDash([4, 2]);
        ctx.beginPath();
        ctx.moveTo(p1.x, p1.y);
        ctx.lineTo(p2.x, p2.y);
        ctx.stroke();
        ctx.setLineDash([]);

        // End points
        ctx.fillStyle = strokeColor;
        ctx.beginPath();
        ctx.arc(p1.x, p1.y, 2.5, 0, Math.PI * 2);
        ctx.arc(p2.x, p2.y, 2.5, 0, Math.PI * 2);
        ctx.fill();

        // Label if sufficiently zoomed in
        if (state.zoom > 10) {
            const midX = (p1.x + p2.x) / 2;
            const midY = (p1.y + p2.y) / 2;
            const text = `! ${v.dist.toFixed(3)} mm`;

            ctx.font = "bold 9px monospace";
            const tw = ctx.measureText(text).width;
            ctx.fillStyle = "rgba(15, 23, 42, 0.9)";
            ctx.fillRect(midX - tw / 2 - 3, midY - 12, tw + 6, 14);
            ctx.strokeStyle = strokeColor;
            ctx.lineWidth = 1;
            ctx.strokeRect(midX - tw / 2 - 3, midY - 12, tw + 6, 14);

            ctx.fillStyle = strokeColor;
            ctx.textAlign = "center";
            ctx.textBaseline = "middle";
            ctx.fillText(text, midX, midY - 5);
        }
    }
    ctx.restore();
}

function renderZoneDrawingPreview() {
    const p0 = worldToScreen(
        Math.min(state.drawStartWorld.x, state.drawCurrentWorld.x),
        Math.max(state.drawStartWorld.y, state.drawCurrentWorld.y)
    );
    const p1 = worldToScreen(
        Math.max(state.drawStartWorld.x, state.drawCurrentWorld.x),
        Math.min(state.drawStartWorld.y, state.drawCurrentWorld.y)
    );
    const sw = Math.abs(p1.x - p0.x);
    const sh = Math.abs(p1.y - p0.y);

    ctx.fillStyle = "rgba(124, 58, 237, 0.25)";
    ctx.fillRect(p0.x, p0.y, sw, sh);
    ctx.strokeStyle = "#7c3aed";
    ctx.lineWidth = 2;
    ctx.setLineDash([4, 4]);
    ctx.strokeRect(p0.x, p0.y, sw, sh);
    ctx.setLineDash([]);
}

function updateCoordsDisplay() {
    const el = document.getElementById("coordsDisplay");
    const zPct = Math.round(state.zoom * 100 / 3.0);
    el.innerText = `Zoom: ${zPct}% | Apertures: ${state.apertures.length}`;
}

// -------------------------------------------------------------
// Interactive Selection & Actions
// -------------------------------------------------------------
async function selectAperture(apId) {
    state.selectedApertureId = apId;
    try {
        const res = await fetch(`/api/aperture/${apId}`);
        if (!res.ok) return;
        const ap = await res.json();
        state.selectedAperture = ap;

        updateInspectorUI(ap);
        if (ap.package_name && ap.package_name !== state.highlightPkg) {
            setPackageHighlight(ap.package_name, null);
        }
        const badge = document.getElementById("padPkgNameBadge");
        if (badge) badge.textContent = ap.package_name || ap.package_group || "-";
        render();
    } catch (e) {
        console.error("Fehler beim Laden der Apertur-Details:", e);
    }
}

// Größte zulässige Dicke für das gewählte Pad und Vorschlag (Standardschritt, Stufenzone)
function updateThicknessRecommendation(ap) {
    const box = document.getElementById("thickRecBox");
    if (!box || !ap.t_max) return;
    const tMax = Math.round(ap.t_max * 1000), step = Math.round(ap.t_step * 1000);
    const cur = Math.round(ap.ipc.effective_thickness * 1000);
    const bad = cur > tMax;
    box.classList.toggle("bad", bad);
    document.getElementById("thickRecVal").innerText = tMax >= 1000 ? "not critical" : `≤ ${tMax} µm → ${step} µm`;
    document.getElementById("thickRecNote").innerText = bad
        ? `At ${cur} µm, ${cur - tMax} µm too thick (Area/Aspect Ratio below limit). Fix: step zone ${step} µm, thinner base stencil or larger aperture.`
        : `OK: ${cur} µm ≤ ${tMax} µm (margin ${tMax - cur} µm).`;
    const btn = document.getElementById("btnZoneAroundComp");
    const canZone = bad && ap.comp_id && step < cur;
    btn.classList.toggle("hidden", !canZone);
    if (canZone) {
        btn.innerText = `▣ Step zone ${step} µm around ${ap.comp_id}`;
        btn.onclick = async () => {
            const res = await fetch("/api/zone/around_component", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ component: ap.comp_id, thickness: step / 1000 }) });
            const d = await res.json();
            if (!res.ok) { showToast(`${d.message || d.error}`); return; }
            await refreshApertureData();
            await refreshHistory();
            if (state.selectedApertureId != null) await selectAperture(state.selectedApertureId);
            showToast(`Step zone ${step} µm around ${ap.comp_id} created (Ctrl+Z to undo)`);
        };
    }
}

// Übersicht je Standarddicke (Pads außerhalb von Stufenzonen) mit begrenzenden Packages
async function openThicknessOverview() {
    const pop = document.getElementById("thickPopover");
    if (!pop) return;
    if (!pop.classList.contains("hidden")) { pop.classList.add("hidden"); return; }
    const d = await (await fetch("/api/thickness/overview")).json();
    const cur = Math.round(d.current * 1000), rec = Math.round(d.recommended_ok * 1000), noCrit = Math.round(d.recommended * 1000);
    const rows = d.steps.map(s => {
        const t = Math.round(s.t * 1000);
        return `<tr class="${t === cur ? "cur" : ""} ${t === rec ? "rec" : ""}" data-t="${t}" style="cursor:pointer" title="Apply ${t} µm">
            <td>${t} µm</td><td>${s.ok}</td><td>${s.warn}</td><td class="${s.crit ? "crit" : ""}">${s.crit}</td></tr>`;
    }).join("");
    const lim = (d.limiting || []).slice(0, 6).map(l => `<div><span>${escapeHtml(l.package)}</span><span>≤ ${Math.round(l.t_max * 1000)} µm · ${l.pads_below}/${l.pads} Pads</span></div>`).join("");
    pop.innerHTML = `<div><strong>Base Stencil Thickness</strong> – ${d.pads} pads outside step zones</div>
        <table><tr><th>Thickness</th><th><span class="dot ok"></span></th><th><span class="dot warn"></span></th><th><span class="dot crit"></span></th></tr>${rows}</table>
        <div>★ Recommended (all pads green): <strong>${rec} µm</strong>${noCrit > rec ? ` · max. ${noCrit} µm without red pads` : ""}</div>
        <div style="opacity:.75">Click a row to apply the thickness</div>
        ${lim ? `<div class="lim"><div><strong>Limiting packages at ${cur} µm</strong></div>${lim}<div>→ Better use step zones for these than a thinner base stencil</div></div>` : ""}`;
    pop.querySelectorAll("tr[data-t]").forEach(tr => tr.addEventListener("click", (e) => {
        e.stopPropagation();
        const el = document.getElementById("inputBaseThickness");
        el.value = tr.dataset.t;
        el.dispatchEvent(new Event("change"));
        pop.classList.add("hidden");
    }));
    pop.classList.remove("hidden");
}

function updateInspectorUI(ap) {
    document.getElementById("noSelectionMsg").classList.add("hidden");
    document.getElementById("selectionDetails").classList.remove("hidden");

    document.getElementById("padId").innerText = ap.id;
    document.getElementById("padDim").innerText = `${ap.width.toFixed(2)} x ${ap.height.toFixed(2)} mm`;
    document.getElementById("padPos").innerText = `X: ${ap.cx.toFixed(2)} | Y: ${ap.cy.toFixed(2)}`;

    const thickUm = Math.round(ap.ipc.effective_thickness * 1000);
    const zoneText = ap.ipc.zone_id ? ` (Zone)` : ` (base)`;
    document.getElementById("padThick").innerText = `${thickUm} µm${zoneText}`;
    document.getElementById("padIdentical").innerText = `${ap.identical_count} on board`;
    document.querySelectorAll(".lblIdenticalCount").forEach(el => el.innerText = ap.identical_count);

    const badge = document.getElementById("padStatusBadge");
    badge.className = `badge ${ap.ipc.status.toLowerCase()}`;
    badge.innerText = ap.ipc.status;

    updateThicknessRecommendation(ap);

    const ar = ap.ipc.area_ratio;
    document.getElementById("padArVal").innerText = ar.toFixed(3);
    const fill = document.getElementById("gaugeBarFill");
    fill.className = `gauge-bar-fill ${ap.ipc.status.toLowerCase()}`;
    const fillPct = Math.min(100, Math.max(0, (ar / 1.0) * 100));
    fill.style.width = `${fillPct}%`;

    const clAlert = document.getElementById("clearanceAlert");
    if (ap.ipc.clearance_warn) {
        clAlert.classList.remove("hidden");
    } else {
        clAlert.classList.add("hidden");
    }

    const grAlert = document.getElementById("grainAlert");
    if (grAlert) {
        if (ap.ipc && ap.ipc.grain_warn) {
            grAlert.classList.remove("hidden");
            const msgEl = document.getElementById("grainAlertMsg");
            if (msgEl) msgEl.innerText = ap.ipc.grain_msg || "Aperture narrower than minimum for paste type!";
        } else {
            grAlert.classList.add("hidden");
        }
    }
    for (const [id, on, msg] of [["via", ap.ipc?.via_warn, ap.ipc?.via_msg], ["squeegee", ap.ipc?.squeegee_warn, ap.ipc?.squeegee_msg]]) {
        document.getElementById(`${id}Alert`)?.classList.toggle("hidden", !on);
        const el = document.getElementById(`${id}AlertMsg`);
        if (el) el.innerText = msg || "";
    }
}

// -------------------------------------------------------------
// Auslösekalkulator Integration
// -------------------------------------------------------------
async function runCalculator(w, h, unit) {
    const res = await fetch("/api/calculator", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ width: w, height: h, unit: unit })
    });
    const data = await res.json();
    renderCalculatorResults(data);
}

function renderCalculatorResults(data) {
    const grid = document.getElementById("calcResultsGrid");
    grid.innerHTML = "";

    const unit = data.input_unit;
    const wVal = unit === "mil" ? data.width_mil : data.width_mm;
    const hVal = unit === "mil" ? data.height_mil : data.height_mm;

    for (const r of data.results) {
        const card = document.createElement("div");
        card.className = "calc-excel-card";

        const arClass = r.status.toLowerCase();
        const sideClass = r.status === "OK" ? "" : (r.status === "WARN" ? "warn" : "crit");

        const areaStr = unit === "mil" ? `${r.area_mil2.toFixed(2)}` : `${r.area_mm2.toFixed(4)}`;
        const wallStr = unit === "mil" ? `${r.wall_mil2.toFixed(2)}` : `${r.wall_mm2.toFixed(4)}`;
        const volStr = unit === "mil" ? `${r.volume_mil3.toFixed(2)} mil³` : `${r.volume_nl.toFixed(3)} nl`;

        card.innerHTML = `
            <div class="calc-excel-main">
                <div class="calc-excel-header">${r.thickness_um} µm</div>
                <div class="calc-excel-row split">
                    <span>${wVal}</span>
                    <span>x</span>
                    <span>${hVal}</span>
                </div>
                <div class="calc-excel-row">
                    <span>Area:</span>
                    <span>${areaStr}</span>
                </div>
                <div class="calc-excel-row">
                    <span>Wall area:</span>
                    <span>${wallStr}</span>
                </div>
                <div class="calc-excel-row highlight-ar ${arClass}">
                    <span>Area Ratio (AR):</span>
                    <span>${r.area_ratio.toFixed(2)}</span>
                </div>
                <div class="calc-excel-row">
                    <span>Volume:</span>
                    <span>${volStr}</span>
                </div>
            </div>
            <div class="calc-excel-side ${sideClass}">
                ${r.transfer_pct}%
            </div>
        `;
        grid.appendChild(card);
    }
}

// -------------------------------------------------------------
// Multi-Layer Manager & Native Windows Import
// -------------------------------------------------------------
async function loadLayers() {
    try {
        const [resLayers, resPrims] = await Promise.all([
            fetch("/api/layers"),
            fetch("/api/layer_primitives?all=1")
        ]);
        const dataLayers = await resLayers.json();
        const dataPrims = await resPrims.json();

        state.layers = dataLayers.layers || [];
        state.layerPrimitives = dataPrims.layers || {};
        refreshSides();

        renderLayerTable();
        render();
    } catch (e) {
        console.error("Fehler beim Laden der Lagen:", e);
    }
}

// Gruppen im Lagen-Manager (Reihenfolge = Anzeige). Dielektrika/Aufbau sind standardmäßig eingeklappt.
const LAYER_GROUPS = [
    { key: "paste", label: "Paste / Stencil", test: (l) => ["paste", "paste_orig", "zone"].includes(l.type) || isOdbStencil(l) },
    { key: "assembly", label: "Components", test: (l) => ["component", "fiducial"].includes(l.type) },
    { key: "copper", label: "Copper", test: (l) => l.type === "copper" },
    { key: "mask", label: "Solder Mask", test: (l) => l.type === "mask" },
    { key: "silk", label: "Silkscreen", test: (l) => l.type === "silk" },
    { key: "drill", label: "Drills", test: (l) => l.type === "drill" },
    { key: "profile", label: "Outline", test: (l) => l.type === "profile" },
    { key: "stackup", label: "Stackup", collapsed: true, test: (l) => /dielectric|polyimide|prepreg|core|coverlay|cover(top|bottom)|stiffener|bend_area|flex_area|rigid_area/i.test(`${l.name} ${l.filename || ""}`) },
    { key: "copies", label: "Copies", collapsed: true, test: (l) => /_copy$/i.test(l.name) },
    { key: "other", label: "Other", test: () => true },
];
// Stencil-Lage aus dem ODB (nicht unsere top_stencil / bot_stencil)
const isOdbStencil = (l) => l.type !== "paste" && /stencil/i.test(l.name) && !/_copy$/i.test(l.name);
const layerUi = { search: "", filter: "all", collapsed: null };

function layerGroupOf(layer) {
    // Aufbau-Lagen zuerst prüfen, damit Coverlay/Stiffener nicht unter "Sonstige" landen
    if (/_copy$/i.test(layer.name)) return LAYER_GROUPS.find(g => g.key === "copies");
    const stack = LAYER_GROUPS.find(g => g.key === "stackup");
    if (!["copper", "paste", "paste_orig", "component", "profile", "drill"].includes(layer.type) && stack.test(layer)) return stack;
    return LAYER_GROUPS.find(g => g.key !== "stackup" && g.test(layer));
}

function layerMatchesFilter(layer) {
    const q = layerUi.search.trim().toLowerCase();
    if (q && !`${layer.name} ${layer.filename || ""}`.toLowerCase().includes(q)) return false;
    switch (layerUi.filter) {
        case "copper": return layer.type === "copper";
        case "mask": return layer.type === "mask";
        case "paste": return layer.type === "paste" || layer.type === "paste_orig";
        case "visible": return layer.visible !== false;
        default: return true;
    }
}

function setupLayerFilters() {
    try { layerUi.collapsed = JSON.parse(localStorage.getItem("sc.layerGroups") || "null"); } catch (e) { layerUi.collapsed = null; }
    // neue Gruppen bekommen ihren Standard (eingeklappt/offen), gespeicherte bleiben
    layerUi.collapsed = { ...Object.fromEntries(LAYER_GROUPS.map(g => [g.key, Boolean(g.collapsed)])), ...(layerUi.collapsed || {}) };
    document.getElementById("layerSearch")?.addEventListener("input", (e) => { layerUi.search = e.target.value; renderLayerTable(); });
    document.querySelectorAll("#layerFilterChips button[data-f]").forEach(btn => btn.addEventListener("click", () => {
        layerUi.filter = btn.dataset.f;
        document.querySelectorAll("#layerFilterChips button[data-f]").forEach(b => b.classList.toggle("active", b === btn));
        renderLayerTable();
    }));
    document.querySelectorAll("#layerFilterChips button[data-side]").forEach(btn => btn.addEventListener("click", () => switchLayerSide(btn.dataset.side)));
}

// Generator-Schalter ↔ Lagen der aktiven Seite (Bohrungen beidseitig)
const PREVIEW_LAYERS = {
    chkPrevCopper: l => l.type === "copper",
    chkPrevPaste: l => l.type === "paste",
    chkPrevMask: l => l.type === "mask",
    chkPrevCustomer: l => l.type === "paste_orig",
    chkPrevComponent: l => l.type === "component" && /^comp/i.test(l.name),
    chkPrevDrills: l => l.type === "drill",
};
const previewSideLayers = (test) => state.layers.filter(l => test(l) && (l.side === (state.side || "top") || (l.type === "drill" && l.side === "both")));
function syncPreviewFromLayers() {
    for (const [id, test] of Object.entries(PREVIEW_LAYERS)) {
        const chk = document.getElementById(id);
        const ls = previewSideLayers(test);
        if (!chk || !ls.length) continue;
        const on = ls.some(l => l.visible !== false);
        if (chk.checked !== on) { chk.checked = on; chk.dispatchEvent(new Event("change")); }
    }
}

// TOP/BOT-Schnellschalter: nur Sichtbarkeit von COMP, Stencil (unsere Paste) und Solder; keine Ansicht, kein Spiegeln
const isSideSwitchLayer = l => l.type === "paste" || l.type === "mask" || (l.type === "component" && /^comp/i.test(l.name));
function switchLayerSide(side) {
    for (const l of state.layers) {
        if (isSideSwitchLayer(l) && (l.side === "top" || l.side === "bot")) setLayerVisible(l, l.side === side);
    }
    renderLayerTable();
    render();
}
function updateSideSwitch() {
    const set = state.layers.filter(l => isSideSwitchLayer(l) && (l.side === "top" || l.side === "bot"));
    for (const b of document.querySelectorAll("#layerFilterChips button[data-side]")) {
        b.classList.toggle("active", set.length > 0 && set.every(l => (l.visible !== false) === (l.side === b.dataset.side)));
    }
}

function setLayerVisible(layer, checked) {
    layer.visible = checked;
    if (state.layerPrimitives[layer.id]) state.layerPrimitives[layer.id].visible = checked;
    fetch(`/api/layer/${layer.id}/toggle`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ visible: checked })
    });
}

function renderLayerTable() {
    const tbody = document.getElementById("layerTableBody");
    if (!tbody) return;
    tbody.innerHTML = "";
    if (!layerUi.collapsed) setupLayerFilters();

    const countBadge = document.getElementById("layerCountBadge");
    if (countBadge) countBadge.innerText = state.layers.length;
    updateSideSwitch();
    syncPreviewFromLayers();

    // Bei Suche/Filter alle passenden Gruppen aufgeklappt zeigen
    const filtering = layerUi.search.trim() !== "" || layerUi.filter !== "all";
    let shown = 0;
    for (const g of LAYER_GROUPS) {
        const all = state.layers.filter(l => layerGroupOf(l) === g);
        const members = all.filter(layerMatchesFilter);
        if (members.length === 0) continue;
        const collapsed = !filtering && layerUi.collapsed[g.key];
        const visibleCount = all.filter(l => l.visible !== false).length;

        const gr = document.createElement("tr");
        gr.className = "layer-group-row";
        gr.innerHTML = `<td colspan="5"><div class="layer-group-head">
            <span class="grp-caret">${collapsed ? "▸" : "▾"}</span>
            <span>${g.label}</span>
            <span class="grp-count">${members.length}${visibleCount ? ` · ${visibleCount} visible` : ""}</span>
            <button class="grp-eye" title="${visibleCount ? "Hide all layers of group" : "Show all layers of group"}">${visibleCount ? icon("eye-closed") : icon("eye")}</button>
        </div></td>`;
        gr.addEventListener("click", () => {
            layerUi.collapsed[g.key] = !layerUi.collapsed[g.key];
            localStorage.setItem("sc.layerGroups", JSON.stringify(layerUi.collapsed));
            renderLayerTable();
        });
        gr.querySelector(".grp-eye").addEventListener("click", (e) => {
            e.stopPropagation();
            const on = visibleCount === 0;
            for (const l of members) setLayerVisible(l, on);
            renderLayerTable();
            render();
        });
        tbody.appendChild(gr);
        if (!collapsed) {
            for (const layer of members) tbody.appendChild(layerRow(layer));
        }
        shown += members.length;
    }
    if (shown === 0 && state.layers.length > 0) {
        const tr = document.createElement("tr");
        tr.className = "layer-empty-row";
        tr.innerHTML = `<td colspan="5">No layer matches search/filter</td>`;
        tbody.appendChild(tr);
    }
}

const LAYER_TYPE_LABEL = { paste: "Paste", paste_orig: "Paste (Customer)", component: "Component", copper: "Copper", mask: "Mask", drill: "Drill", zone: "Zone", fiducial: "Fiducial", profile: "Profile", silk: "Silk", other: "Misc" };

function layerRow(layer) {
        const tr = document.createElement("tr");
        if (!layer.visible) tr.classList.add("row-hidden");

        const isVisible = layer.visible !== false;
        const color = layer.color || "#c084fc";
        const typeBadge = layer.type ? layer.type.toLowerCase() : "other";

        tr.innerHTML = `
            <td class="col-chk">
                <input type="checkbox" class="layer-chk-input" data-layer-id="${layer.id}" ${isVisible ? 'checked' : ''} title="Show/hide layer">
            </td>
            <td class="col-color">
                <input type="color" class="layer-color-picker" data-layer-id="${layer.id}" value="${color}" title="Change color">
            </td>
            <td class="layer-name-cell" title="${escapeHtml(layer.name)} (${escapeHtml(layer.filename || '')})">${escapeHtml(layer.name)}${isOdbStencil(layer) ? ' <span class="layer-src" title="Stencil layer from the ODB++ job, not generated here">ODB</span>' : ""}</td>
            <td class="col-info">${escapeHtml([layer.info, layer.count ? `Obj: ${layer.count.toLocaleString()}` : "", LAYER_TYPE_LABEL[typeBadge] || layer.type || ""].filter(Boolean).join("; "))}</td>
            <td class="col-del">
                <button class="layer-del-btn" data-layer-id="${layer.id}" title="Remove layer">✕</button>
            </td>
        `;

        // Sichtbarkeit: sofort zeichnen, im Hintergrund synchronisieren; Gruppenzähler aktualisieren
        const chk = tr.querySelector(".layer-chk-input");
        chk.addEventListener("change", (e) => {
            setLayerVisible(layer, e.target.checked);
            renderLayerTable();
            render();
        });

        // Color Picker Event -> Sofortiges Canvas-Update!
        const cp = tr.querySelector(".layer-color-picker");
        cp.addEventListener("input", (e) => {
            layer.color = e.target.value;
            if (state.layerPrimitives[layer.id]) {
                state.layerPrimitives[layer.id].color = e.target.value;
            }
            render();
        });
        cp.addEventListener("change", (e) => {
            fetch(`/api/layer/${layer.id}/update`, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ color: e.target.value })
            });
        });

        // Delete Layer Event
        const delBtn = tr.querySelector(".layer-del-btn");
        delBtn.addEventListener("click", async (e) => {
            e.stopPropagation();
            if (confirm(`Remove layer "${layer.name}"?`)) {
                await fetch(`/api/layer/${layer.id}`, { method: "DELETE" });
                delete state.layerPrimitives[layer.id];
                state.layers = state.layers.filter(l => l.id !== layer.id);
                renderLayerTable();
                render();
            }
        });

        return tr;
}

// Kundenbibliothek speichern; bei Fehler Meldung zeigen und false liefern
async function saveLibrary(customer) {
    try {
        const res = await fetch("/api/library/save", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ customer }) });
        const d = await res.json();
        if (res.ok && d.status === "ok") return true;
        showToast(`Library "${customer}" not saved: ${d.message || res.status}`, 8000);
    } catch (e) {
        showToast(`Library "${customer}" not saved: ${e}`, 8000);
    }
    return false;
}

function escapeHtml(text) {
    if (!text) return "";
    return String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function showToast(msg, durationMs = 3500) {
    const existing = document.querySelector(".app-toast");
    if (existing) existing.remove();

    const toast = document.createElement("div");
    toast.className = "app-toast";
    toast.innerHTML = `${icon("info", "toast-ic")} <span>${msg}</span>`;
    document.body.appendChild(toast);

    if (durationMs > 0) {
        setTimeout(() => {
            toast.style.transition = "opacity 0.3s ease";
            toast.style.opacity = "0";
            setTimeout(() => toast.remove(), 300);
        }, durationMs);
    }
    return toast;
}

async function promptCloseBeforeLoad() {
    // Nur bei ungespeicherten Änderungen nachfragen
    return guardUnsaved("loading a new dataset");
}

// Windows Dialog Triggers
async function triggerWindowsFileDialog(clearPrevious = true) {
    if (clearPrevious) {
        if (!await promptCloseBeforeLoad()) return;
    }
    const toast = showToast("Windows file dialog open... Please select files in Explorer.", 0);
    try {
        const res = await fetch("/api/open_windows_file_dialog", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ side: "top", clear_previous: clearPrevious })
        });
        toast.remove();
        const data = await res.json();
        if (data.status === "ok") {
            showToast(`${data.count} layer(s) imported successfully!`);
            await loadLayers();
            await refreshApertureData();
            fitToScreen();
            document.getElementById("zipModal")?.classList.add("hidden");
        } else if (data.status === "cancelled") {
            showToast("File selection cancelled");
        } else {
            showToast(`Error: ${data.message || 'Unknown'}`);
        }
    } catch (e) {
        toast.remove();
        showToast("Error opening Windows file dialog");
    }
}

async function triggerWindowsFolderDialog() {
    if (!await promptCloseBeforeLoad()) return;
    const toast = showToast("Windows folder dialog open... Please select a folder in Explorer.", 0);
    try {
        const res = await fetch("/api/open_windows_folder_dialog", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ side: "top" })
        });
        toast.remove();
        const data = await res.json();
        if (data.status === "ok") {
            showToast(`Folder imported (${data.count} layers)!`);
            await loadLayers();
            await refreshApertureData();
            fitToScreen();
            document.getElementById("zipModal")?.classList.add("hidden");
        } else if (data.status === "cancelled") {
            showToast("Folder selection cancelled");
        } else {
            showToast(`Error: ${data.message || 'Unknown'}`);
        }
    } catch (e) {
        toast.remove();
        showToast("Error opening Windows folder dialog");
    }
}

async function uploadFilesFromBrowser(files) {
    if (!files || files.length === 0) return;
    if (!await promptCloseBeforeLoad()) return;
    const toast = showToast(`Uploading ${files.length} file(s)...`, 0);
    try {
        const formData = new FormData();
        for (let i = 0; i < files.length; i++) {
            formData.append("files", files[i]);
        }
        formData.append("side", "top");

        const res = await fetch("/api/upload_layers", {
            method: "POST",
            body: formData
        });
        toast.remove();
        const data = await res.json();
        if (data.status === "ok") {
            showToast(`${data.count} layer(s) imported!`);
            await loadLayers();
            await refreshApertureData();
            fitToScreen();
        } else {
            showToast(`Error: ${data.message}`);
        }
    } catch (e) {
        toast.remove();
        showToast("Upload failed");
    }
}

// -------------------------------------------------------------
// Event Listeners (Mouse, Zoom, Pan, Actions)
// -------------------------------------------------------------
function setupEventListeners() {
    container.addEventListener("wheel", (e) => {
        e.preventDefault();
        const rect = container.getBoundingClientRect();
        const mouseX = e.clientX - rect.left;
        const mouseY = e.clientY - rect.top;

        const worldBefore = screenToWorld(mouseX, mouseY);
        const zoomFactor = e.deltaY < 0 ? 1.2 : 0.833;
        state.zoom = Math.max(0.2, Math.min(150.0, state.zoom * zoomFactor));

        state.panX = mouseX - worldBefore.x * state.zoom;
        state.panY = rect.height - mouseY - worldBefore.y * state.zoom;

        render();
    }, { passive: false });

    container.addEventListener("mousedown", (e) => {
        if (e.target !== canvas) return; // Klicks auf Buttons/Overlays ignorieren
        // Rechte Maustaste verschiebt die Ansicht (wie im Stencil Generator)
        if (e.button === 2) {
            state.isDragging = true;
            state.rightPan = true;
            state.dragStartX = e.clientX;
            state.dragStartY = e.clientY;
            return;
        }
        if (e.button === 0 || e.button === 1) {
            if (measureState.active && e.button === 0) {
                const rect = container.getBoundingClientRect();
                const sx = e.clientX - rect.left;
                const sy = e.clientY - rect.top;
                let worldPt = screenToWorld(sx, sy);
                const snap = e.shiftKey ? null : findSnapPointBoard(worldPt.x, worldPt.y);
                if (snap) {
                    worldPt = { x: snap.x, y: snap.y };
                }

                if (!measureState.startPoint) {
                    measureState.startPoint = worldPt;
                    measureState.currentPoint = worldPt;
                } else {
                    const endPt = worldPt;
                    const dx = endPt.x - measureState.startPoint.x;
                    const dy = endPt.y - measureState.startPoint.y;
                    const dist = Math.hypot(dx, dy);
                    if (dist > 0.005) {
                        measureState.measurements.push({
                            id: Date.now(),
                            mode: "board",
                            start: measureState.startPoint,
                            end: endPt,
                            dist: dist,
                            dx: dx,
                            dy: dy
                        });
                        updateMeasureButtonsVisibility();
                    }
                    measureState.startPoint = null;
                    measureState.currentPoint = null;
                }
                render();
                return;
            }

            const zsel = state.selectedZoneId && (state.zones || []).find(z => z.id === state.selectedZoneId);
            if (zsel && e.button === 0 && !state.isDrawingZone) {
                const rect = container.getBoundingClientRect();
                const sx = e.clientX - rect.left, sy = e.clientY - rect.top;
                const h = zoneHandles(zsel).find(h => Math.abs(h.x - sx) <= 7 && Math.abs(h.y - sy) <= 7);
                const w = screenToWorld(sx, sy);
                const inside = w.x >= zsel.x0 && w.x <= zsel.x1 && w.y >= zsel.y0 && w.y <= zsel.y1;
                if (h || inside) {
                    state.zoneEdit = { z: zsel, mode: h ? h.k : "move", start: w, orig: { x0: zsel.x0, y0: zsel.y0, x1: zsel.x1, y1: zsel.y1 }, moved: false };
                    return;
                }
            }
            if (e.button === 0 && !state.isDrawingZone && (selState.addMode || e.shiftKey)) {
                const rect = container.getBoundingClientRect();
                const w = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
                if (selState.addMode) { placeNewOpening(w); return; }
                selState.box = { start: w, cur: w };
                return;
            }
            if (state.isDrawingZone) {
                const rect = container.getBoundingClientRect();
                state.drawStartWorld = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
                state.drawCurrentWorld = state.drawStartWorld;
            } else {
                state.isDragging = true;
                state.dragStartX = e.clientX;
                state.dragStartY = e.clientY;
            }
        }
    });

    window.addEventListener("mousemove", (e) => {
        const rect = container.getBoundingClientRect();
        if (state.zoneEdit) {
            const ze = state.zoneEdit, o = ze.orig;
            const w = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
            const dx = w.x - ze.start.x, dy = w.y - ze.start.y;
            const z = ze.z;
            if (ze.mode === "move") { z.x0 = o.x0 + dx; z.x1 = o.x1 + dx; z.y0 = o.y0 + dy; z.y1 = o.y1 + dy; }
            else {
                // Bildschirm "oben" = größeres Y
                if (ze.mode.includes("w")) z.x0 = Math.min(o.x0 + dx, o.x1 - 0.2);
                if (ze.mode.includes("e")) z.x1 = Math.max(o.x1 + dx, o.x0 + 0.2);
                if (ze.mode.includes("n")) z.y1 = Math.max(o.y1 + dy, o.y0 + 0.2);
                if (ze.mode.includes("s")) z.y0 = Math.min(o.y0 + dy, o.y1 - 0.2);
            }
            ze.moved = true;
            render();
            return;
        }
        if (selState.box) {
            selState.box.cur = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
            render();
            return;
        }
        if (state.isDragging) {
            const dx = e.clientX - state.dragStartX;
            const dy = e.clientY - state.dragStartY;
            state.panX += dx;
            state.panY -= dy;
            state.dragStartX = e.clientX;
            state.dragStartY = e.clientY;
            render();
        } else if (state.isDrawingZone && state.drawStartWorld) {
            state.drawCurrentWorld = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
            render();
        } else if (measureState.active && currentWorkspaceMode === "board") {
            const sx = e.clientX - rect.left;
            const sy = e.clientY - rect.top;
            if (sx >= 0 && sx <= rect.width && sy >= 0 && sy <= rect.height) {
                const rawWorld = screenToWorld(sx, sy);
                measureState.hoverSnap = e.shiftKey ? null : findSnapPointBoard(rawWorld.x, rawWorld.y);
                measureState.currentPoint = measureState.hoverSnap ? { x: measureState.hoverSnap.x, y: measureState.hoverSnap.y } : rawWorld;
                render();
            }
        } else {
            const w = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
            document.getElementById("coordsDisplay").innerText = 
                `X: ${w.x.toFixed(2)} mm | Y: ${w.y.toFixed(2)} mm | Zoom: ${Math.round(state.zoom * 100 / 3.0)}%`;
        }
    });

    // Doppelklick auf Package oder Pad -> Sofortiger Sprung in den Stencil Generator
    container.addEventListener("dblclick", (e) => {
        if (e.target !== canvas || measureState.active) return;
        const rect = container.getBoundingClientRect();
        const sx = e.clientX - rect.left;
        const sy = e.clientY - rect.top;
        const w = screenToWorld(sx, sy);

        let targetPkg = null;
        let targetPin = null;
        let targetRot = 0;

        // 1. Suche nach Apertur (Paste / Pad): nächstgelegenes Pad im Fangbereich
        if (state.apertures) {
            const hits = state.apertures
                .filter(ap => Math.abs(ap.cx - w.x) <= ap.w / 2 + 0.3 && Math.abs(ap.cy - w.y) <= ap.h / 2 + 0.3)
                .sort((a, b) => Math.hypot(a.cx - w.x, a.cy - w.y) - Math.hypot(b.cx - w.x, b.cy - w.y));
            for (const ap of hits) {
                {
                    targetPkg = ap.pkg_name || ap.package_name || ap.package || ap.pkg;
                    targetPin = ap.pin_number ?? ap.pin;
                    if (ap.comp_id && state.components) {
                        const comp = state.components.find(c => c.refdes === ap.comp_id);
                        if (comp) targetRot = comp.rot || comp.rotation || 0;
                    }
                    break;
                }
            }
        }

        // 2. Suche nach Bauteil (Components)
        if (!targetPkg && state.components) {
            for (const c of state.components) {
                const halfW = (c.width || 2.0) / 2 + 0.5;
                const halfH = (c.height || 2.0) / 2 + 0.5;
                if (Math.abs(c.cx - w.x) <= halfW && Math.abs(c.cy - w.y) <= halfH) {
                    targetPkg = c.package || c.pkg_name;
                    targetRot = c.rot || c.rotation || 0;
                    break;
                }
            }
        }

        // 3. Fallback: Nächste Apertur im Umkreis von 4 mm
        if (!targetPkg && state.apertures) {
            let closestDist = Infinity;
            for (const ap of state.apertures) {
                const dist = Math.hypot(ap.cx - w.x, ap.cy - w.y);
                if (dist < closestDist && dist < 4.0) {
                    closestDist = dist;
                    targetPkg = ap.pkg_name || ap.package_name || ap.package || ap.pkg;
                    targetPin = ap.pin_number ?? ap.pin;
                    if (ap.comp_id && state.components) {
                        const comp = state.components.find(c => c.refdes === ap.comp_id);
                        if (comp) targetRot = comp.rot || comp.rotation || 0;
                    }
                }
            }
        }

        if (targetPkg && window.scGeneratorDetached) {
            window.scBroadcast?.("open-package", { pkg: targetPkg, pin: targetPin ?? null, rot: targetRot });
            window.scFocusGenerator?.();
            setPackageHighlight(targetPkg, targetPin ?? null);
            return;
        }
        if (targetPkg) {
            switchWorkspace("package", targetPkg, targetRot);
            showToast(`Package "${targetPkg}" opened in Stencil Generator (${targetRot}°)`);
            if (targetPin !== null && targetPin !== undefined) {
                setTimeout(() => {
                    if (pkgState.activePackageData && pkgState.activePackageData.pads) {
                        const pad = pkgState.activePackageData.pads.find(p => p.pin === targetPin);
                        if (pad) selectPkgPin(targetPin, pad);
                    }
                }, 200);
            }
        }
    });

    // Rechtsklick auf Platine -> Messungen sofort löschen
    container.addEventListener("contextmenu", (e) => {
        if (state.rightPanMoved) { state.rightPanMoved = false; e.preventDefault(); return; }
        if ((measureState.measurements && measureState.measurements.length > 0) || measureState.active) {
            e.preventDefault();
            clearMeasurements();
            if (measureState.active) toggleMeasureTool(false);
        }
    });

    // Measurement Tool Toolbar & Banner Controls
    document.getElementById("btnMeasureBoard")?.addEventListener("click", () => toggleMeasureTool());
    document.getElementById("btnCancelMeasureBoard")?.addEventListener("click", () => toggleMeasureTool(false));
    document.getElementById("btnClearMeasurementsBoard")?.addEventListener("click", clearMeasurements);
    document.getElementById("btnClearMeasureBoard")?.addEventListener("click", clearMeasurements);

    // Global Key Listener für [M], [ESC], [DEL], [C]
    window.addEventListener("keydown", (e) => {
        const tag = document.activeElement ? document.activeElement.tagName.toLowerCase() : "";
        if (tag === "input" || tag === "textarea" || tag === "select") return;

        if ((e.ctrlKey || e.metaKey) && !e.altKey) {
            const k = e.key.toLowerCase();
            if (k === "z" && !e.shiftKey) { e.preventDefault(); doUndo(); return; }
            if (k === "y" || (k === "z" && e.shiftKey)) { e.preventDefault(); doRedo(); return; }
            if (k === "s") { e.preventDefault(); saveProject(e.shiftKey); return; }
        }
        if (currentWorkspaceMode === "board" && handleSelectionKey(e)) {
            e.preventDefault();
            return;
        }
        // Stencil Generator: Pfeil rechts klappt die Pins des Packages auf, links wieder zu (wie im Explorer)
        if (currentWorkspaceMode !== "board" && (e.key === "ArrowRight" || e.key === "ArrowLeft") && !e.defaultPrevented && pkgState.activePkgName) {
            const open = e.key === "ArrowRight";
            if (pkgState.pinsExpanded !== open) {
                e.preventDefault();
                if (!open && pkgState.pinBarMode !== "all") openAllPinsBar();
                pkgState.pinsExpanded = open;
                localStorage.setItem("sc.pinsOpen", open ? "1" : "0");
                renderPkgTree();
            }
            return;
        }
        // Stencil Generator: Pfeil hoch/runter geht in der Package-Liste zum vorigen/nächsten Package
        if (currentWorkspaceMode !== "board" && (e.key === "ArrowUp" || e.key === "ArrowDown") && !e.defaultPrevented) {
            const items = [...document.querySelectorAll("#pkgTreeList .pkg-tree-item")];
            const i = items.findIndex(el => el.classList.contains("active"));
            const next = items[Math.min(items.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)))];
            if (next && next !== items[i]) {
                e.preventDefault();
                next.click();
                next.scrollIntoView({ block: "nearest" });
            }
            return;
        }
        if (e.key === "m" || e.key === "M") {
            e.preventDefault();
            toggleMeasureTool();
        } else if (e.key === "Escape") {
            if (measureState.startPoint) {
                measureState.startPoint = null;
                measureState.currentPoint = null;
                render();
                renderPkgPreview();
            } else if (measureState.measurements.length > 0) {
                clearMeasurements();
            } else if (measureState.active) {
                toggleMeasureTool(false);
            }
        } else if (e.key === "Delete" || e.key === "Backspace" || e.key === "c" || e.key === "C") {
            if (measureState.measurements.length > 0) {
                clearMeasurements();
            }
        }
    });

    window.addEventListener("mouseup", (e) => {
        if (state.zoneEdit) {
            const ze = state.zoneEdit;
            state.zoneEdit = null;
            if (ze.moved) saveZoneGeometry(ze.z);
            return;
        }
        if (selState.box) {
            const b = selState.box;
            selState.box = null;
            const tiny = Math.abs(b.cur.x - b.start.x) * state.zoom < 4 && Math.abs(b.cur.y - b.start.y) * state.zoom < 4;
            if (tiny) {
                // Umschalt-Klick = einzelnes Pad zur Auswahl hinzufügen/entfernen
                const rect = container.getBoundingClientRect();
                handleCanvasClick(e.clientX - rect.left, e.clientY - rect.top, { toggle: true });
            } else {
                selectInBox(b.start, b.cur);
            }
            render();
            return;
        }
        if (state.isDragging && state.rightPan) {
            // Rechtes Ziehen: nur verschieben, kein Klick; das Kontextmenü danach unterdrücken
            state.rightPanMoved = Math.hypot(e.clientX - state.dragStartX, e.clientY - state.dragStartY) >= 4;
            state.isDragging = false;
            state.rightPan = false;
            return;
        }
        if (state.isDragging) {
            const dist = Math.hypot(e.clientX - state.dragStartX, e.clientY - state.dragStartY);
            state.isDragging = false;
            if (dist < 4) {
                const rect = container.getBoundingClientRect();
                // Klick außerhalb der gewählten Zone hebt die Zonenauswahl auf
                if (state.selectedZoneId) { state.selectedZoneId = null; renderZonesList(); }
                handleCanvasClick(e.clientX - rect.left, e.clientY - rect.top, { toggle: e.ctrlKey || e.metaKey });
            }
        } else if (state.isDrawingZone && state.drawStartWorld) {
            const w0 = state.drawStartWorld;
            const w1 = state.drawCurrentWorld;
            state.isDrawingZone = false;
            state.drawStartWorld = null;
            state.drawCurrentWorld = null;
            document.getElementById("drawingBanner").classList.add("hidden");

            if (Math.abs(w1.x - w0.x) > 1 && Math.abs(w1.y - w0.y) > 1) {
                openZoneModal(w0.x, w0.y, w1.x, w1.y);
            }
            render();
        }
    });

    function handleCanvasClick(sx, sy, opts = {}) {
        const w = screenToWorld(sx, sy);
        let bestItem = null;
        let minDist = 1.0;

        for (const ap of state.apertures) {
            const dx = Math.abs(ap.cx - w.x);
            const dy = Math.abs(ap.cy - w.y);
            if (dx <= (ap.w / 2 + 0.3) && dy <= (ap.h / 2 + 0.3)) {
                const dist = Math.hypot(dx, dy);
                if (dist < minDist) {
                    minDist = dist;
                    bestItem = ap;
                }
            }
        }

        if (!bestItem) {
            const other = state.otherSide && stencilLayerOf(state.otherSide);
            const hit = other?.visible && (state.otherApertures || []).find(ap => Math.abs(ap.cx - w.x) <= ap.w / 2 + 0.3 && Math.abs(ap.cy - w.y) <= ap.h / 2 + 0.3);
            if (hit) {
                const side = state.otherSide;
                switchSide(side, { silent: true }).then(() => {
                    const ap = state.apertures.find(a => Math.abs(a.cx - hit.cx) < 1e-3 && Math.abs(a.cy - hit.cy) < 1e-3);
                    if (ap) { setSelection([ap.id]); selectAperture(ap.id); }
                });
                return;
            }
            // Klick ins Leere hebt die Mehrfachauswahl auf
            if (!opts.toggle) setSelection([]);
            return;
        }
        if (opts.toggle) {
            toggleSelection(bestItem.id);
        } else {
            setSelection([bestItem.id]);
        }
        if (selState.ids.has(bestItem.id)) selectAperture(bestItem.id);
    }

    // Header Controls
    document.getElementById("datasetSelect")?.addEventListener("change", (e) => {
        loadDataset(e.target.value);
    });

    document.getElementById("btnOpenDataset")?.addEventListener("click", () => triggerWindowsFileDialog(true));
    document.getElementById("btnAddMoreLayers")?.addEventListener("click", () => triggerWindowsFileDialog(false));

    document.getElementById("ipcModeSelect")?.addEventListener("change", async (e) => {
        const mode = e.target.value;
        const cls = /^class(\d)$/.exec(mode);
        if (cls) {
            await fetch("/api/ipc/class", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ class: +cls[1] }) });
        } else {
            await fetch("/api/set_ipc_mode", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ mode }) });
        }
        await refreshApertureData();
        await refreshHistory();
    });
    document.getElementById("btnIpcProfiles")?.addEventListener("click", openIpcProfiles);
    document.getElementById("btnTombApplyAll")?.addEventListener("click", () => applyTombstone([]));
    document.getElementById("btnTombSaveCfg")?.addEventListener("click", () => saveTombstoneConfig());
    document.getElementById("chkTombAuto")?.addEventListener("change", () => saveTombstoneConfig());
    document.getElementById("btnIpcCancel")?.addEventListener("click", () => document.getElementById("ipcProfileModal").classList.add("hidden"));
    document.getElementById("btnIpcReset")?.addEventListener("click", async () => {
        const d = await (await fetch("/api/ipc/profiles/reset", { method: "POST" })).json();
        renderIpcProfileTable(d.profiles);
    });
    document.getElementById("btnIpcSave")?.addEventListener("click", saveIpcProfiles);

    // Color Mode
    document.getElementById("colorModeSelect")?.addEventListener("change", (e) => {
        state.colorMode = e.target.value; render();
    });

    // Layer Manager Toolbar & Windows Dialogs
    document.getElementById("btnWinImportFile")?.addEventListener("click", () => triggerWindowsFileDialog(true));
    document.getElementById("btnWinImportFolder")?.addEventListener("click", triggerWindowsFolderDialog);
    document.getElementById("btnModalWinFile")?.addEventListener("click", () => triggerWindowsFileDialog(true));
    document.getElementById("btnModalWinFolder")?.addEventListener("click", triggerWindowsFolderDialog);

    document.getElementById("browserFileInput")?.addEventListener("change", (e) => {
        uploadFilesFromBrowser(e.target.files);
        e.target.value = "";
    });

    document.getElementById("btnAllLayersOn")?.addEventListener("click", async () => {
        for (const l of state.layers) l.visible = true;
        renderLayerTable();
        render();
        await fetch("/api/layers/toggle_all", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ visible: true })
        });
    });

    document.getElementById("btnAllLayersOff")?.addEventListener("click", async () => {
        for (const l of state.layers) l.visible = false;
        renderLayerTable();
        render();
        await fetch("/api/layers/toggle_all", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ visible: false })
        });
    });

    document.getElementById("btnRefreshLayers")?.addEventListener("click", async () => {
        await loadLayers();
        showToast("Layers updated");
    });

    document.getElementById("layerGlobalOpacity")?.addEventListener("input", (e) => {
        state.globalLayerOpacity = parseFloat(e.target.value);
        const lbl = document.getElementById("layerGlobalOpacityVal");
        if (lbl) lbl.innerText = `${Math.round(state.globalLayerOpacity * 100)}%`;
        render();
    });

    // Drag & Drop on Dropzone and Canvas
    const dropzone = document.getElementById("layerDropZone");
    if (dropzone) {
        dropzone.addEventListener("dragover", (e) => {
            e.preventDefault();
            dropzone.classList.add("dragover");
        });
        dropzone.addEventListener("dragleave", () => {
            dropzone.classList.remove("dragover");
        });
        dropzone.addEventListener("drop", (e) => {
            e.preventDefault();
            dropzone.classList.remove("dragover");
            uploadFilesFromBrowser(e.dataTransfer.files);
        });
        dropzone.addEventListener("click", () => {
            document.getElementById("browserFileInput")?.click();
        });
    }

    container.addEventListener("dragover", (e) => {
        e.preventDefault();
    });
    container.addEventListener("drop", (e) => {
        e.preventDefault();
        if (e.dataTransfer.files && e.dataTransfer.files.length > 0) {
            uploadFilesFromBrowser(e.dataTransfer.files);
        }
    });

    // Optional Legacy Layer Toggles if present
    document.getElementById("chkPaste")?.addEventListener("change", (e) => {
        state.showPaste = e.target.checked; render();
    });
    document.getElementById("chkPads")?.addEventListener("change", (e) => {
        state.showPads = e.target.checked; render();
    });
    document.getElementById("chkComponents")?.addEventListener("change", (e) => {
        state.showComponents = e.target.checked; render();
    });
    document.getElementById("chkZones")?.addEventListener("change", (e) => {
        state.showZones = e.target.checked; render();
    });
    document.getElementById("chkProfile")?.addEventListener("change", (e) => {
        state.showProfile = e.target.checked; render();
    });
    document.getElementById("chkFiducials")?.addEventListener("change", (e) => {
        state.showFiducials = e.target.checked; render();
    });

    // Zoom Toolbar
    const zoomAroundCenter = (factor) => {
        const rect = container.getBoundingClientRect();
        const cx = rect.width / 2, cy = rect.height / 2;
        const before = screenToWorld(cx, cy);
        state.zoom = Math.max(0.2, Math.min(150.0, state.zoom * factor));
        state.panX = cx - before.x * state.zoom;
        state.panY = rect.height - cy - before.y * state.zoom;
        render();
    };
    document.getElementById("btnZoomIn").addEventListener("click", () => zoomAroundCenter(1.3));
    document.getElementById("btnZoomOut").addEventListener("click", () => zoomAroundCenter(1 / 1.3));
    document.getElementById("btnFitScreen").addEventListener("click", fitToScreen);

    // Rakelrichtung (Stufenkanten-Abstandsprüfung längs / quer zur Druckrichtung)
    document.getElementById("squeegeeDir")?.addEventListener("change", async (e) => {
        await fetch("/api/set_squeegee", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ dir: e.target.value })
        });
        await refreshApertureData();
    });

    // Sidebar Tabs
    document.querySelectorAll(".tab-btn").forEach(btn => {
        btn.addEventListener("click", () => {
            document.querySelectorAll(".tab-btn").forEach(b => b.classList.remove("active"));
            document.querySelectorAll(".tab-content").forEach(c => c.classList.remove("active"));
            btn.classList.add("active");
            document.getElementById(btn.dataset.tab).classList.add("active");
        });
    });

    // Preset Grid Buttons (2x2, 3x3)
    document.querySelectorAll(".btn-preset").forEach(btn => {
        btn.addEventListener("click", () => {
            document.querySelectorAll(".btn-preset").forEach(b => b.classList.remove("active"));
            btn.classList.add("active");
        });
    });

    // ACTION: LÄNGLICH & SCHMAL
    document.getElementById("btnApplyElongate").addEventListener("click", async () => {
        if (!state.selectedAperture) return;
        const widthRed = parseFloat(document.getElementById("elongateWidthRed").value);
        const lengthUse = parseFloat(document.getElementById("elongateLengthUse").value);
        const radius = parseFloat(document.getElementById("elongateRadius").value);
        const applyAll = document.getElementById("chkElongateAll").checked;

        await fetch("/api/elongate", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                id: state.selectedAperture.id,
                width_reduction: widthRed,
                length_usage: lengthUse,
                corner_radius: radius,
                apply_all: applyAll
            })
        });
        await refreshApertureData();
    });

    // ACTION: APPLY SLICING
    document.getElementById("btnApplySlice").addEventListener("click", async () => {
        if (!state.selectedAperture) return;
        const activePreset = document.querySelector(".btn-preset.active").dataset.grid;
        const [gx, gy] = activePreset.split("x").map(Number);
        const web = parseFloat(document.getElementById("sliceWeb").value);
        const edge = parseFloat(document.getElementById("sliceEdge").value);
        const applyAll = document.getElementById("chkSliceAll").checked;

        await fetch("/api/slice", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                id: state.selectedAperture.id,
                grid_x: gx,
                grid_y: gy,
                web_width: web,
                edge_reduction: edge,
                apply_all: applyAll
            })
        });
        await refreshApertureData();
    });

    // ACTION: APPLY REDUCTION
    document.getElementById("btnApplyReduce").addEventListener("click", async () => {
        if (!state.selectedAperture) return;
        const pct = parseFloat(document.getElementById("reducePercent").value);
        const applyAll = document.getElementById("chkReduceAll").checked;

        await fetch("/api/reduce", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                id: state.selectedAperture.id,
                percent: pct,
                apply_all: applyAll
            })
        });
        await refreshApertureData();
    });

    // ACTION: RESET
    document.getElementById("btnResetAperture").addEventListener("click", async () => {
        if (!state.selectedAperture) return;
        await fetch("/api/reset_aperture", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                id: state.selectedAperture.id,
                apply_all: false
            })
        });
        await refreshApertureData();
    });

    // ZONE DRAWING & MANAGEMENT
    document.getElementById("btnNewZone").addEventListener("click", startDrawingZone);
    document.getElementById("btnAddZoneTab").addEventListener("click", startDrawingZone);
    document.getElementById("btnCancelDrawing").addEventListener("click", () => {
        state.isDrawingZone = false;
        document.getElementById("drawingBanner").classList.add("hidden");
    });

    // ZONE PILLS (FREIE ZAHLENEINGABE)
    document.querySelectorAll(".pill-btn").forEach(p => {
        p.addEventListener("click", () => {
            document.querySelectorAll(".pill-btn").forEach(b => b.classList.remove("active"));
            p.classList.add("active");
            document.getElementById("modalZoneThickUm").value = p.dataset.thick;
        });
    });

    // AUSLÖSEKALKULATOR EVENTS
    const calcModal = document.getElementById("calculatorModal");
    document.getElementById("btnOpenCalculator").addEventListener("click", () => {
        calcModal.classList.remove("hidden");
        const w = parseFloat(document.getElementById("calcWidth").value) || 11.8;
        const h = parseFloat(document.getElementById("calcHeight").value) || 11.8;
        runCalculator(w, h, state.calcUnit);
    });
    document.getElementById("btnCloseCalcModal").addEventListener("click", () => {
        calcModal.classList.add("hidden");
    });
    document.getElementById("btnSendToCalc").addEventListener("click", () => {
        if (!state.selectedAperture) return;
        const ap = state.selectedAperture;
        calcModal.classList.remove("hidden");

        if (state.calcUnit === "mil") {
            document.getElementById("calcWidth").value = (ap.width / 0.0254).toFixed(1);
            document.getElementById("calcHeight").value = (ap.height / 0.0254).toFixed(1);
        } else {
            document.getElementById("calcWidth").value = ap.width.toFixed(3);
            document.getElementById("calcHeight").value = ap.height.toFixed(3);
        }
        runCalculator(parseFloat(document.getElementById("calcWidth").value),
                      parseFloat(document.getElementById("calcHeight").value),
                      state.calcUnit);
    });
    document.querySelectorAll(".btn-unit").forEach(b => {
        b.addEventListener("click", () => {
            document.querySelectorAll(".btn-unit").forEach(u => u.classList.remove("active"));
            b.classList.add("active");
            state.calcUnit = b.dataset.unit;
            // Werte umrechnen
            let w = parseFloat(document.getElementById("calcWidth").value);
            let h = parseFloat(document.getElementById("calcHeight").value);
            if (state.calcUnit === "mil") {
                document.getElementById("calcWidth").value = (w / 0.0254).toFixed(1);
                document.getElementById("calcHeight").value = (h / 0.0254).toFixed(1);
            } else {
                document.getElementById("calcWidth").value = (w * 0.0254).toFixed(3);
                document.getElementById("calcHeight").value = (h * 0.0254).toFixed(3);
            }
            runCalculator(parseFloat(document.getElementById("calcWidth").value),
                          parseFloat(document.getElementById("calcHeight").value),
                          state.calcUnit);
        });
    });
    document.getElementById("btnRecalc").addEventListener("click", () => {
        const w = parseFloat(document.getElementById("calcWidth").value);
        const h = parseFloat(document.getElementById("calcHeight").value);
        runCalculator(w, h, state.calcUnit);
    });

    // PASTENTYP (IPC-7525 5-BALL REGEL)
    document.getElementById("selectPasteType")?.addEventListener("change", async (e) => {
        const ptype = e.target.value;
        try {
            const res = await fetch("/api/set_paste_type", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ paste_type: ptype })
            });
            const data = await res.json();
            state.pasteType = data.paste_type;
            showToast(`Paste type changed to ${data.paste_type.toUpperCase()} (5-ball rule active)`);
            await refreshApertureData();
        } catch (err) {
            console.error("Fehler beim Wechsel des Pastentyps:", err);
        }
    });

    // SCHABLONENSTEG-PRÜFUNG (BRIDGE / WEB CHECK)
    const bridgeModal = document.getElementById("bridgeModal");
    document.getElementById("btnCheckBridges")?.addEventListener("click", () => {
        if (bridgeModal) bridgeModal.classList.remove("hidden");
        runBridgeCheck();
    });
    document.getElementById("btnCloseBridgeModal")?.addEventListener("click", () => bridgeModal?.classList.add("hidden"));
    document.getElementById("btnCloseBridgeModalBtn")?.addEventListener("click", () => bridgeModal?.classList.add("hidden"));
    document.getElementById("btnRunBridgeCheckNow")?.addEventListener("click", () => {
        const minB = parseFloat(document.getElementById("inputMinBridgeVal").value) || 0.10;
        runBridgeCheck(minB);
    });

    // PIN-IN-PASTE (PiP) MODAL & CONTROLS
    const pipModal = document.getElementById("pipModal");
    document.getElementById("btnOpenPipModal")?.addEventListener("click", () => {
        if (pipModal) {
            pipModal.classList.remove("hidden");
            if (state.selectedAperture) {
                document.getElementById("pipTargetInfo").innerText = `Target: aperture #${state.selectedAperture.id} (${state.selectedAperture.width.toFixed(2)} × ${state.selectedAperture.height.toFixed(2)} mm)`;
            } else {
                document.getElementById("pipTargetInfo").innerText = `Note: no pad selected (preview mode)`;
            }
            calculatePip();
        }
    });
    document.getElementById("btnClosePipModal")?.addEventListener("click", () => pipModal?.classList.add("hidden"));
    document.getElementById("btnCancelPip")?.addEventListener("click", () => pipModal?.classList.add("hidden"));

    document.getElementById("pipPinShape")?.addEventListener("change", (e) => {
        const isRect = e.target.value === "rect";
        const gH = document.getElementById("groupPipPinH");
        if (gH) gH.style.display = isRect ? "block" : "none";
        const lbl = document.getElementById("lblPipPinDim");
        if (lbl) lbl.innerText = isRect ? "Pin width (mm):" : "Pin-Ø (mm):";
        calculatePip();
    });

    [
        "pipDrill", "pipPinW", "pipPinH", "pipPcbThick", "pipMetalRatio", "pipMeniscus", "pipStencilThick"
    ].forEach(id => {
        document.getElementById(id)?.addEventListener("input", calculatePip);
    });

    document.querySelectorAll("[data-pip-shape]").forEach(btn => {
        btn.addEventListener("click", () => {
            document.querySelectorAll("[data-pip-shape]").forEach(b => b.classList.remove("active"));
            btn.classList.add("active");
            state.pipShape = btn.dataset.pipShape;
            calculatePip();
        });
    });

    document.getElementById("btnApplyPipAperture")?.addEventListener("click", async () => {
        if (state.selectedApertureId === null) {
            alert("Please select an aperture / pad on the board first!");
            return;
        }
        await applyPipToAperture();
    });

    // ZIP MODAL EVENTS (Fallback)
    const zipModal = document.getElementById("zipModal");
    document.getElementById("btnOpenZipModal")?.addEventListener("click", () => {
        zipModal?.classList.remove("hidden");
    });
    document.getElementById("btnCancelZipModal")?.addEventListener("click", () => {
        zipModal?.classList.add("hidden");
    });
    document.getElementById("btnConfirmLoadZip")?.addEventListener("click", async () => {
        if (!await promptCloseBeforeLoad()) return;
        const path = document.getElementById("inputZipPath").value;
        const side = document.getElementById("selectZipSide").value;
        const res = await fetch("/api/load_zip", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ path, side })
        });
        const data = await res.json();
        if (data.status === "ok") {
            zipModal.classList.add("hidden");
            await loadLayers();
            await refreshApertureData();
            fitToScreen();
            showToast("ZIP dataset loaded!");
        } else {
            alert("Error loading ZIP: " + data.message);
        }
    });

    // EXPORT
    document.getElementById("btnExport").addEventListener("click", openExportDialog);
    setupExportDialog();

    // Datensatz schließen (mit Abfrage bei ungespeicherten Änderungen)
    document.getElementById("btnCloseDataset")?.addEventListener("click", async () => {
        if (!(await guardUnsaved("closing the dataset"))) return;
        await fetch("/api/project/discard_recovery", { method: "POST" });
        await closeCurrentDataset();
    });
}

async function triggerExport() {
    try {
        const res = await fetch("/api/export", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
        const data = await res.json();
        showToast("Gerber files exported!");
        return data;
    } catch (e) {
        console.error("Fehler beim Export:", e);
    }
}

async function closeCurrentDataset() {
    try {
        const res = await fetch("/api/close_dataset", { method: "POST" });
        const data = await res.json();
        state.layers = [];
        state.layerPrimitives = {};
        state.apertures = [];
        state.pads = [];
        state.components = [];
        state.zones = [];
        state.profile = [];
        state.fiducials = [];
        state.bounds = [0, 0, 0, 0];
        await loadLayers();
        await refreshApertureData();
        fitToScreen();
        showToast("✕ Dataset closed");
        if (currentWorkspaceMode === "package") {
            await loadPackagesList();
            selectPackage("0805");
        }
    } catch (e) {
        console.error("Fehler beim Schließen des Datensatzes:", e);
    }
}

function startDrawingZone() {
    state.isDrawingZone = true;
    document.getElementById("drawingBanner").classList.remove("hidden");
}

let pendingZoneCoords = null;
function openZoneModal(x0, y0, x1, y1) {
    pendingZoneCoords = { x0, y0, x1, y1 };
    const chk = document.getElementById("modalZoneIsStep");
    if (chk) chk.checked = !state.drawRuleZone;
    state.drawRuleZone = false;
    document.getElementById("zoneModal").classList.remove("hidden");
}

document.getElementById("btnCancelModal").addEventListener("click", () => {
    document.getElementById("zoneModal").classList.add("hidden");
    pendingZoneCoords = null;
});

document.getElementById("btnSaveZoneModal").addEventListener("click", async () => {
    if (!pendingZoneCoords) return;
    const thickUm = parseFloat(document.getElementById("modalZoneThickUm").value);
    const thickMm = thickUm / 1000.0;
    const name = document.getElementById("modalZoneName").value;
    const isStep = document.getElementById("modalZoneIsStep")?.checked !== false;

    await fetch("/api/zone", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            x0: pendingZoneCoords.x0,
            y0: pendingZoneCoords.y0,
            x1: pendingZoneCoords.x1,
            y1: pendingZoneCoords.y1,
            thickness: thickMm,
            name: name,
            step: isStep
        })
    });

    document.getElementById("zoneModal").classList.add("hidden");
    renderZoneRuleList();
    pendingZoneCoords = null;
    await refreshApertureData();
});

// -------------------------------------------------------------
// Side Lists (Zones & Critical)
// -------------------------------------------------------------
function renderZonesList() {
    const list = document.getElementById("zonesList");
    list.innerHTML = "";

    if (!state.zones || state.zones.length === 0) {
        list.innerHTML = `<p class="empty-state">No step zones defined.</p>`;
        return;
    }

    for (const z of state.zones) {
        const card = document.createElement("div");
        card.className = `zone-card ${z.id === state.selectedZoneId ? "selected" : ""}`;
        const thickUm = Math.round(z.thickness * 1000);
        card.innerHTML = `
            <div class="zone-card-info">
                <h4>${escapeHtml(z.name)}</h4>
                <div class="zone-card-meta">${z.width} x ${z.height} mm | Thickness: <strong>${thickUm} µm</strong></div>
            </div>
            <div style="display: flex; gap: 6px;">
                <button class="btn btn-sm btn-secondary btn-edit-thick" data-id="${z.id}" data-thick="${thickUm}">Thickness</button>
                <button class="btn btn-sm btn-danger btn-del-zone" data-id="${z.id}">Delete</button>
            </div>
        `;

        card.querySelector(".btn-edit-thick").addEventListener("click", async (e) => {
            e.stopPropagation();
            const cur = card.querySelector(".btn-edit-thick").dataset.thick;
            const neu = prompt(`New zone thickness for "${z.name}" in µm:`, cur);
            if (neu && !isNaN(parseFloat(neu))) {
                await fetch(`/api/zone/${z.id}`, {
                    method: "POST",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify({ thickness: parseFloat(neu) / 1000.0 })
                });
                await refreshApertureData();
            }
        });

        card.querySelector(".btn-del-zone").addEventListener("click", async (e) => {
            e.stopPropagation();
            await fetch(`/api/zone/${z.id}`, { method: "DELETE" });
            if (state.selectedZoneId === z.id) state.selectedZoneId = null;
            await refreshApertureData();
        });
        card.addEventListener("click", () => selectZone(z.id === state.selectedZoneId ? null : z.id, z.id !== state.selectedZoneId));
        if (z.id === state.selectedZoneId) {
            // Dicke und Maße direkt bearbeiten
            const f = (v) => (+v).toFixed(3);
            const ed = document.createElement("div");
            ed.className = "zone-edit";
            ed.innerHTML = `<label>Thickness µm<input type="number" data-k="t" step="5" value="${thickUm}"></label>
                <label>Width mm<input type="number" data-k="w" step="0.1" value="${f(z.x1 - z.x0)}"></label>
                <label>Height mm<input type="number" data-k="h" step="0.1" value="${f(z.y1 - z.y0)}"></label>
                <label>Center X/Y<input type="text" data-k="c" value="${f((z.x0 + z.x1) / 2)} / ${f((z.y0 + z.y1) / 2)}"></label>
                <div class="full"><button class="btn btn-xs btn-primary">Apply</button></div>`;
            ed.addEventListener("click", (e) => e.stopPropagation());
            ed.querySelector("button").addEventListener("click", async () => {
                const v = (k) => ed.querySelector(`[data-k="${k}"]`).value;
                const [cx, cy] = v("c").split("/").map(s => parseFloat(s.replace(",", ".")));
                const w = parseFloat(v("w")), h = parseFloat(v("h")), t = parseFloat(v("t"));
                if (![cx, cy, w, h, t].every(Number.isFinite) || w <= 0 || h <= 0) { showToast("Invalid values"); return; }
                await saveZoneGeometry({ id: z.id, x0: cx - w / 2, y0: cy - h / 2, x1: cx + w / 2, y1: cy + h / 2, thickness: t / 1000 });
                showToast(`${z.name} changed`);
            });
            card.appendChild(ed);
        }
        list.appendChild(card);
    }
}

// Grabstein-Risiko: Liste, Übernehmen, Einstellungen
const TOMB_SHAPES = { inverted_house_builder: "House, tip outward", house_builder: "House, tip to component", simple_rect: "Rectangle (reduced)" };
let tombState = null;

async function refreshTombstones() {
    try {
        tombState = await (await fetch("/api/tombstone")).json();
    } catch (e) { return; }
    renderTombstones();
}

function renderTombstones() {
    const d = tombState;
    if (!d) return;
    const badge = document.getElementById("tombTabBadge");
    if (badge) { badge.textContent = d.open; badge.classList.toggle("hidden", !d.open); }
    document.getElementById("tombCount").textContent = `${d.open} open / ${d.candidates.length}`;
    document.getElementById("tombRuleLabel").textContent = TOMB_SHAPES[d.config.rule.rule_type] || d.config.rule.rule_type;
    document.getElementById("chkTombAuto").checked = d.config.auto_apply;
    const c = d.config;
    const setv = (id, v) => { const el = document.getElementById(id); if (el && document.activeElement !== el) el.value = v; };
    setv("tombShape", c.rule.rule_type); setv("tombCut", c.rule.house_cut_pct); setv("tombUndersize", c.rule.undersize_w);
    setv("tombRadius", c.rule.corner_radius); setv("tombPour", c.min_pour_area); setv("tombRatio", c.trace_ratio);
    const list = document.getElementById("tombList");
    list.innerHTML = "";
    if (!d.candidates.length) {
        list.innerHTML = `<p class="empty-state" style="padding:8px">${d.copper_data ? "No 2-pin chips with unequal copper connection found." : "No copper data (only available for ODB++ with outer layers)."}</p>`;
        return;
    }
    for (const t of d.candidates) {
        const row = document.createElement("div");
        row.className = `tomb-row ${t.has_rule ? "done" : ""} ${state.focusComp === t.component ? "focused" : ""}`;
        row.title = "Click: show on board";
        const pinInfo = t.pins.map(p => `Pin ${p.pin}${p.pin === t.weak_pin ? " (weak)" : ""}: ${p.pour_area > 0 ? `Pour ${p.pour_area.toFixed(1)} mm²` : `${p.traces} trace(s) ${p.trace_w.toFixed(2)} mm${p.vias ? `, ${p.vias} Via` : ""}`}`).join(" · ");
        const shapes = [...new Set(t.pins.map(p => shapeLabel(p.shape)))].join(" / ");
        row.innerHTML = `<div class="t-head">${escapeHtml(t.component)}<span class="pkg">${escapeHtml(t.package)}</span></div>
            <div class="t-btns">
                ${t.has_rule
                    ? `<span class="ok-tag">${icon("check")} Rule active</span><button class="btn btn-xs btn-ghost" data-act="revert" title="Remove component rule – apertures back to package rule">Undo</button>`
                    : '<button class="btn btn-xs btn-secondary" data-act="apply">Apply</button>'}
            </div>
            <div class="t-reason">${escapeHtml(t.reason)}<br>${escapeHtml(pinInfo)}</div>
            <div class="t-shape">Aperture: <strong>${escapeHtml(shapes)}</strong></div>`;
        row.addEventListener("click", () => focusComponent(t));
        row.querySelector('[data-act="apply"]')?.addEventListener("click", (e) => { e.stopPropagation(); applyTombstone([t.component], t); });
        row.querySelector('[data-act="revert"]')?.addEventListener("click", (e) => { e.stopPropagation(); revertTombstone([t.component], t); });
        list.appendChild(row);
    }
}

// Lesbare Bezeichnung der erzeugten Öffnungsform
function shapeLabel(kind) {
    const k = String(kind || "original");
    if (k.includes("inverted_house")) return "House, tip outward";
    if (k.includes("house")) return "House, tip to component";
    if (k.includes("simple_rect") || k.includes("package_group_rect")) return "Rectangle";
    if (k.includes("bga")) return "Round";
    if (k === "original") return "Original (Customer)";
    return k.replace(/_/g, " ");
}

// Bauteil auf dem Board zeigen und mit Rahmen hervorheben
async function focusComponent(t) {
    state.focusComp = t.component;
    const rect = container.getBoundingClientRect();
    state.zoom = Math.max(state.zoom, 60);
    state.panX = rect.width / 2 - t.x * state.zoom;
    state.panY = rect.height / 2 - t.y * state.zoom;
    await selectAperture(t.pins[0].id);
    renderTombstones();
    render();
}

async function applyTombstone(components, focus = null) {
    const res = await fetch("/api/tombstone/apply", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ components }) });
    const d = await res.json();
    tombState = d.tombstone;
    await refreshApertureData();
    await refreshHistory();
    if (focus) {
        const t = tombState.candidates.find(c => c.component === focus.component) || focus;
        await focusComponent(t);
        showToast(`${t.component}: apertures now "${shapeLabel(t.pins[0].shape)}" (undo in list or Ctrl+Z)`);
    } else {
        renderTombstones();
        showToast(`Tombstoning protection applied to ${d.applied} component(s) (Ctrl+Z to undo)`);
    }
}

async function revertTombstone(components, focus = null) {
    const d = await (await fetch("/api/tombstone/revert", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ components }) })).json();
    tombState = d.tombstone;
    await refreshApertureData();
    await refreshHistory();
    if (focus) {
        const t = tombState.candidates.find(c => c.component === focus.component) || focus;
        await focusComponent(t);
        showToast(`${t.component}: component rule removed, apertures back to "${shapeLabel(t.pins[0].shape)}"`);
    } else {
        renderTombstones();
    }
}

// Rahmen um das in der Grabstein-Liste gewählte Bauteil
function renderFocusComponent() {
    if (!state.focusComp) return;
    const aps = (state.apertures || []).filter(a => a.comp_id === state.focusComp);
    if (!aps.length) return;
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const a of aps) {
        x0 = Math.min(x0, a.cx - a.w / 2); x1 = Math.max(x1, a.cx + a.w / 2);
        y0 = Math.min(y0, a.cy - a.h / 2); y1 = Math.max(y1, a.cy + a.h / 2);
    }
    const m = 0.15;
    const p0 = worldToScreen(x0 - m, y1 + m), p1 = worldToScreen(x1 + m, y0 - m);
    ctx.save();
    ctx.strokeStyle = "#0078d4";
    ctx.lineWidth = 2;
    ctx.setLineDash([6, 4]);
    ctx.strokeRect(p0.x, p0.y, p1.x - p0.x, p1.y - p0.y);
    ctx.setLineDash([]);
    ctx.fillStyle = "#0078d4";
    ctx.font = "600 12px Segoe UI, sans-serif";
    ctx.fillText(state.focusComp, p0.x, p0.y - 5);
    ctx.restore();
}

async function saveTombstoneConfig(extra = {}) {
    const num = (id) => parseFloat(document.getElementById(id).value);
    const cfg = {
        auto_apply: document.getElementById("chkTombAuto").checked,
        min_pour_area: num("tombPour"), trace_ratio: num("tombRatio"),
        rule: {
            rule_type: document.getElementById("tombShape").value,
            name: `Tombstoning protection (${TOMB_SHAPES[document.getElementById("tombShape").value]})`,
            house_cut_pct: num("tombCut"), undersize_w: num("tombUndersize"), undersize_h: num("tombUndersize"), corner_radius: num("tombRadius"),
        },
        ...extra,
    };
    const d = await (await fetch("/api/tombstone/config", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(cfg) })).json();
    tombState = d.tombstone;
    renderTombstones();
    if (document.getElementById("chkTombSaveLib").checked) {
        const cust = (await (await fetch("/api/libraries")).json()).current_customer || "Standard";
        if (await saveLibrary(cust)) showToast(`Tombstoning settings saved to "${cust}"`);
    } else {
        showToast("Tombstoning settings applied");
    }
}

function renderCriticalList() {
    refreshTombstones();
    const list = document.getElementById("criticalList");
    list.innerHTML = "";

    if (!state.stats || !state.stats.top_critical_packages || state.stats.top_critical_packages.length === 0) {
        list.innerHTML = `<p class="empty-state">No IPC violations found! All apertures release reliably. </p>`;
        return;
    }

    for (const p of state.stats.top_critical_packages) {
        const card = document.createElement("div");
        card.className = "critical-card";
        card.innerHTML = `
            <div class="crit-header">
                <span>Package: ${p.group} mm</span>
                <span class="badge crit">${p.crit_count} critical</span>
            </div>
            <div class="crit-meta">${p.total} pads total on the panel. Click to jump.</div>
        `;
        card.addEventListener("click", () => {
            const target = state.apertures.find(a => a.pkg === p.group && a.status === "CRIT");
            if (target) {
                selectAperture(target.id);
                const rect = container.getBoundingClientRect();
                state.zoom = 8.0;
                state.panX = rect.width / 2 - target.cx * state.zoom;
                state.panY = rect.height / 2 - target.cy * state.zoom;
                render();
            }
        });
        list.appendChild(card);
    }
}


// =============================================================
// WORKSPACE MODE SWITCHER (BOARD VIEW vs STENCIL GENERATOR)
// =============================================================
let currentWorkspaceMode = "board"; // 'board' or 'package'

const btnViewBoard = document.getElementById("btnViewBoard");
const btnViewPackage = document.getElementById("btnViewPackage");
const boardWorkspace = document.getElementById("boardWorkspace");
const packageWorkspace = document.getElementById("packageWorkspace");

btnViewBoard.addEventListener("click", () => switchWorkspace("board"));
btnViewPackage.addEventListener("click", () => switchWorkspace("package"));

const btnOpenInStencilGen = document.getElementById("btnOpenInStencilGen");
if (btnOpenInStencilGen) {
    btnOpenInStencilGen.addEventListener("click", () => {
        const pkg = state.selectedAperture ? (state.selectedAperture.package_name || state.selectedAperture.pkg_name || "0805") : "0805";
        switchWorkspace("package", pkg);
    });
}

function switchWorkspace(mode, targetPkg = null, targetRot = 0) {
    currentWorkspaceMode = mode;
    document.body.classList.toggle("ws-package", mode === "package");
    // Eine angefangene Messung gehört zum alten Koordinatensystem
    measureState.startPoint = null;
    measureState.currentPoint = null;
    measureState.hoverSnap = null;
    updateMeasureButtonsVisibility();
    if (mode === "board") {
        btnViewBoard.classList.add("active");
        btnViewPackage.classList.remove("active");
        boardWorkspace.classList.remove("hidden");
        packageWorkspace.classList.add("hidden");
        resizeCanvas();
        render();
    } else {
        btnViewPackage.classList.add("active");
        btnViewBoard.classList.remove("active");
        packageWorkspace.classList.remove("hidden");
        boardWorkspace.classList.add("hidden");
        initPackageMode(targetPkg, targetRot);
    }
}


// =============================================================
// STENCIL GENERATOR CONTROLLER (PACKAGE-MODUS)
// =============================================================
const pkgState = {
    packages: [],
    activePkgName: "0805",
    activePackageData: null,
    activeCategory: "",
    searchQuery: "",
    currentCustomer: "Standard",
    selectedPin: null,
    rotation: 0, // 0, 90, 180, 270 Grad

    // Transform fuer Package Canvas
    zoom: 60.0, // Pixel pro mm
    panX: 0,
    panY: 0,
    isDragging: false,
    dragStartX: 0,
    dragStartY: 0,

    // Layer Toggles
    showMask: true,
    showCopper: true,
    showPaste: true,
    showComponent: true,
    showRuler: true,
    showPinLabels: false
};

const pkgCanvas = document.getElementById("pkgPreviewCanvas");
const pkgCtx = pkgCanvas ? pkgCanvas.getContext("2d") : null;
const pkgContainer = document.getElementById("pkgPreviewContainer");

let pkgInitialized = false;

async function initPackageMode(targetPkg = null, targetRot = 0) {
    if (!pkgInitialized) {
        setupPkgCanvas();
        setupPkgEventListeners();
        pkgInitialized = true;
    }
    resizePkgCanvas();

    if (targetRot !== undefined && targetRot !== null) {
        pkgState.rotation = targetRot;
    } else {
        pkgState.rotation = 0;
    }

    await loadLibraries();
    await loadPackagesList();

    const selectName = targetPkg || pkgState.activePkgName || "0805";
    selectPackage(selectName);
}

function setupPkgCanvas() {
    if (!pkgCanvas) return;
    resizePkgCanvas();
    window.addEventListener("resize", () => {
        if (currentWorkspaceMode === "package") {
            resizePkgCanvas();
            renderPkgPreview();
        }
    });

    // Mouse Pan & Zoom
    pkgContainer.addEventListener("wheel", (e) => {
        e.preventDefault();
        // Zoom zur Mausposition: der Punkt unter dem Mauszeiger bleibt stehen
        const rect = pkgCanvas.getBoundingClientRect();
        const anchor = pkgScreenToWorld(e.clientX, e.clientY);
        const factor = e.deltaY < 0 ? 1.15 : 0.85;
        pkgState.zoom = Math.max(10, Math.min(400, pkgState.zoom * factor));
        const s = pkgWorldToScreen(anchor.x, anchor.y);
        pkgState.panX += (e.clientX - rect.left) - s.x;
        pkgState.panY += (e.clientY - rect.top) - s.y;
        renderPkgPreview();
    }, { passive: false });

    pkgContainer.addEventListener("mousedown", (e) => {
        if (e.target !== pkgCanvas) return; // Pin-Leiste, Banner, Buttons nicht als Canvas-Klick werten
        // In Package-Koordinaten umrechnen (mm) unter Berücksichtigung der Ansichtsdrehung
        const wpt = pkgScreenToWorld(e.clientX, e.clientY);
        let mmX = wpt.x;
        let mmY = wpt.y;

        // Messmodus in Stencil Generator (Fangpunkt beim Klick neu bestimmen, Shift = ohne Fangen)
        if (measureState.active && e.button === 0) {
            const snap = e.shiftKey ? null : findSnapPointPkg(mmX, mmY);
            if (snap) {
                mmX = snap.x;
                mmY = snap.y;
            }
            if (!measureState.startPoint) {
                measureState.startPoint = { x: mmX, y: mmY };
                measureState.currentPoint = { x: mmX, y: mmY };
            } else {
                const endPt = { x: mmX, y: mmY };
                const mdx = endPt.x - measureState.startPoint.x;
                const mdy = endPt.y - measureState.startPoint.y;
                const dist = Math.hypot(mdx, mdy);
                if (dist > 0.005) {
                    measureState.measurements.push({
                        id: Date.now(),
                        mode: "package",
                        start: measureState.startPoint,
                        end: endPt,
                        dist: dist,
                        dx: mdx,
                        dy: mdy
                    });
                    updateMeasureButtonsVisibility();
                }
                measureState.startPoint = null;
                measureState.currentPoint = null;
            }
            renderPkgPreview();
            return;
        }

        if (!pkgState.spaceDown && sketchMouseDown(e, mmX, mmY)) return;

        // Prüfe ob Pad geklickt wurde
        let clickedPad = null;
        if (pkgState.activePackageData && pkgState.activePackageData.pads) {
            for (const p of pkgState.activePackageData.pads) {
                if (Math.abs(mmX - p.cx) <= p.w / 2 && Math.abs(mmY - p.cy) <= p.h / 2) {
                    clickedPad = p;
                    break;
                }
            }
        }

        const panButton = e.button === 1 || e.button === 2 || (e.button === 0 && pkgState.spaceDown);
        if (panButton) {
            e.preventDefault();
            pkgState.isDragging = true;
            pkgState.panMoved = false;
            pkgState.dragStartX = e.clientX - pkgState.panX;
            pkgState.dragStartY = e.clientY - pkgState.panY;
            return;
        }
        if (e.button !== 0) return;
        const additive = e.ctrlKey || e.shiftKey;
        const current = pkgState.pinBarMode === "multi" ? pkgState.selectedPins : (pkgState.selectedPin !== null ? [pkgState.selectedPin] : []);
        if (clickedPad) {
            if (additive) {
                selectPkgPins(current.includes(clickedPad.pin) ? current.filter(x => x !== clickedPad.pin) : [...current, clickedPad.pin]);
            } else {
                selectPkgPin(clickedPad.pin, clickedPad);
                renderPkgTree();
            }
        } else {
            // Auswahlrahmen aufziehen
            const r = pkgCanvas.getBoundingClientRect();
            pkgState.box = { x0: e.clientX - r.left, y0: e.clientY - r.top, x1: e.clientX - r.left, y1: e.clientY - r.top, add: additive, base: current };
        }
    });
    pkgContainer.addEventListener("auxclick", (e) => { if (e.button === 1) e.preventDefault(); });
    window.addEventListener("keydown", (e) => {
        if (e.code !== "Space" || currentWorkspaceMode !== "package") return;
        const tag = (document.activeElement?.tagName || "").toLowerCase();
        if (tag === "input" || tag === "select" || tag === "textarea") return;
        pkgState.spaceDown = true;
        if (pkgCanvas) pkgCanvas.style.cursor = "grab";
        e.preventDefault();
    });
    window.addEventListener("keyup", (e) => {
        if (e.code === "Space") { pkgState.spaceDown = false; if (pkgCanvas) pkgCanvas.style.cursor = ""; }
    });

    window.addEventListener("mousemove", (e) => {
        if (currentWorkspaceMode === "package" && !pkgState.isDragging && sketchMouseMove(e)) return;
        if (pkgState.box && pkgCanvas) {
            const r = pkgCanvas.getBoundingClientRect();
            pkgState.box.x1 = e.clientX - r.left;
            pkgState.box.y1 = e.clientY - r.top;
            renderPkgPreview();
        } else if (pkgState.isDragging) {
            pkgState.panMoved = true;
            pkgState.panX = e.clientX - pkgState.dragStartX;
            pkgState.panY = e.clientY - pkgState.dragStartY;
            renderPkgPreview();
        } else if (measureState.active && currentWorkspaceMode === "package" && pkgCanvas) {
            const rect = pkgCanvas.getBoundingClientRect();
            const clickX = e.clientX - rect.left;
            const clickY = e.clientY - rect.top;
            if (clickX >= 0 && clickX <= rect.width && clickY >= 0 && clickY <= rect.height) {
                const { x: mmX, y: mmY } = pkgScreenToWorld(e.clientX, e.clientY);
                measureState.hoverSnap = e.shiftKey ? null : findSnapPointPkg(mmX, mmY);
                measureState.currentPoint = measureState.hoverSnap ? { x: measureState.hoverSnap.x, y: measureState.hoverSnap.y } : { x: mmX, y: mmY };
                renderPkgPreview();
            }
        }
    });

    window.addEventListener("mouseup", () => {
        pkgState.isDragging = false;
        if (sketchMouseUp()) return;
        const b = pkgState.box;
        if (!b) return;
        pkgState.box = null;
        const r = pkgCanvas.getBoundingClientRect();
        if (Math.abs(b.x1 - b.x0) < 4 && Math.abs(b.y1 - b.y0) < 4) {
            // Klick ins Leere: Auswahl aufheben (außer mit Strg/Umschalt)
            if (!b.add) { openAllPinsBar(); renderPkgTree(); }
            renderPkgPreview();
            return;
        }
        // Rahmen in Package-Koordinaten (Ecken über die Ansichtsdrehung umrechnen)
        const corners = [[b.x0, b.y0], [b.x1, b.y0], [b.x0, b.y1], [b.x1, b.y1]].map(([x, y]) => pkgScreenToWorld(x + r.left, y + r.top));
        const xs = corners.map(c => c.x), ys = corners.map(c => c.y);
        const [minX, maxX, minY, maxY] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
        const hit = (pkgState.activePackageData?.pads || []).filter(p => p.cx >= minX && p.cx <= maxX && p.cy >= minY && p.cy <= maxY).map(p => p.pin);
        selectPkgPins(b.add ? [...b.base, ...hit] : hit);
        renderPkgPreview();
    });

    // Rechtsklick im Stencil Generator -> Messungen sofort löschen (nicht nach Verschieben mit rechter Taste)
    pkgContainer.addEventListener("contextmenu", (e) => {
        if (pkgState.panMoved) { e.preventDefault(); pkgState.panMoved = false; return; }
        if ((measureState.measurements && measureState.measurements.length > 0) || measureState.active) {
            e.preventDefault();
            clearMeasurements();
            if (measureState.active) toggleMeasureTool(false);
            return;
        }
        if (e.target === pkgCanvas) sketchContextMenu(e);
    });

    // Doppelklick auf Pad im Package-Modus -> Pin auswählen und Eingabe fokussieren
    pkgContainer.addEventListener("dblclick", (e) => {
        if (sk.tool === "poly") { finishPoly(); return; }
        if (e.target !== pkgCanvas || measureState.active || sk.tool) return;
        const { x: mmX, y: mmY } = pkgScreenToWorld(e.clientX, e.clientY);

        if (pkgState.activePackageData && pkgState.activePackageData.pads) {
            for (const p of pkgState.activePackageData.pads) {
                if (Math.abs(mmX - p.cx) <= p.w / 2 && Math.abs(mmY - p.cy) <= p.h / 2) {
                    selectPkgPin(p.pin, p);
                    const inpW = document.getElementById("inputPadCustomW");
                    if (inpW) { inpW.focus(); inpW.select(); }
                    break;
                }
            }
        }
    });
}

function resizePkgCanvas() {
    if (!pkgContainer || !pkgCanvas) return;
    const rect = pkgContainer.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return; // ausgeblendet
    const dpr = window.devicePixelRatio || 1;
    pkgCanvas.width = rect.width * dpr;
    pkgCanvas.height = rect.height * dpr;
    pkgCtx.scale(dpr, dpr);
}

// -------------------------------------------------------------
// Libraries & Packages API
// -------------------------------------------------------------
async function loadLibraries() {
    try {
        const res = await fetch("/api/libraries");
        const data = await res.json();
        const select = document.getElementById("selectCustomerLib");
        select.innerHTML = "";
        for (const lib of data.libraries) {
            const opt = document.createElement("option");
            opt.value = lib;
            opt.textContent = lib === "Standard" ? "Standard (Inhouse)" : lib;
            if (lib === data.current_customer) opt.selected = true;
            select.appendChild(opt);
        }
        pkgState.currentCustomer = data.current_customer;
        pkgState.libraries = data.libraries;
        renderLibHeader();
    } catch (e) {
        console.error("Fehler beim Laden der Bibliotheken:", e);
    }
}

// Kopf der Bibliothek: Name, Änderungsmarke und Laden-Menü (Bibliotheken, vom PC, neu)
function renderLibHeader() {
    const lbl = document.getElementById("lblCurrentLib");
    if (lbl) lbl.textContent = pkgState.currentCustomer === "Standard" ? "Standard (Inhouse)" : (pkgState.currentCustomer || "–");
    document.getElementById("lblLibDirty")?.classList.toggle("hidden", !pkgState.libEdited);
    const list = document.getElementById("libLoadList");
    if (!list) return;
    list.innerHTML = "";
    for (const lib of pkgState.libraries || []) {
        const b = document.createElement("button");
        b.textContent = (lib === pkgState.currentCustomer ? "✓ " : "  ") + (lib === "Standard" ? "Standard (Inhouse)" : lib);
        b.addEventListener("click", () => loadLibraryFlow({ customer: lib }));
        list.appendChild(b);
    }
    list.insertAdjacentHTML("beforeend", '<div class="dropdown-sep"></div>');
    const pc = document.createElement("button");
    pc.textContent = "Load from PC…";
    pc.className = "desktop-only";
    pc.addEventListener("click", () => loadLibraryFlow({ pick: true }));
    if (window.__TAURI__) list.appendChild(pc);
    const nb = document.createElement("button");
    nb.textContent = "＋ New Library…";
    nb.addEventListener("click", () => document.getElementById("newLibModal").classList.remove("hidden"));
    list.appendChild(nb);
}

// Bearbeitung merken: beim ersten Mal nach der Bibliothek fragen, später beim Laden abgleichen
function noteRuleEdit() {
    pkgState.libEdited = true;
    renderLibHeader();
    if (!pkgState.libAsked) {
        pkgState.libAsked = true;
        openLibAsk();
    }
}

function openLibAsk() {
    const m = document.getElementById("libAskModal");
    const list = document.getElementById("libAskList");
    list.innerHTML = "";
    for (const lib of pkgState.libraries || []) {
        const b = document.createElement("button");
        b.className = "btn btn-secondary";
        b.textContent = lib === "Standard" ? "Standard (Inhouse)" : lib;
        if (lib === pkgState.currentCustomer) b.textContent += " (active)";
        b.addEventListener("click", () => { m.classList.add("hidden"); loadLibraryFlow({ customer: lib }); });
        list.appendChild(b);
    }
    document.getElementById("btnLibAskPc").classList.toggle("hidden", !window.__TAURI__);
    m.classList.remove("hidden");
}

// Laden mit Abgleich: hat der Nutzer schon bearbeitet, zeigt ein Dialog die Unterschiede und fragt je Zeile
async function loadLibraryFlow(src) {
    const post = (url, body) => fetch(url, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }).then(r => r.json());
    let keep = [];
    let source = src;
    if (pkgState.libEdited) {
        const d = await post("/api/library/diff", src);
        if (d.status === "cancelled") return;
        if (d.status !== "ok") { showToast(d.error || d.message || "Library not readable", 5000); return; }
        source = d.path ? { path: d.path } : { customer: d.customer };
        if (d.changes.length) {
            keep = await chooseLibDiff(d.customer, d.changes);
            if (keep === null) return;
        }
    }
    const r = await post("/api/library/load", { ...source, keep });
    if (r.status === "cancelled") return;
    if (r.status !== "ok") { showToast(r.error || r.message || "Load failed", 5000); return; }
    pkgState.libEdited = keep.length > 0;
    pkgState.libAsked = true;
    await loadLibraries();
    await loadPackagesList();
    if (pkgState.activePkgName) selectPackage(pkgState.activePkgName);
    refreshHistory();
    showToast(`Library "${r.customer}" loaded${keep.length ? ` – ${keep.length} own versions kept` : ""}`);
}

// Dialog: je Änderung "mein Stand" oder "Bibliothek"; liefert die Liste "behalten" oder null (Abbrechen)
function chooseLibDiff(lib, changes) {
    return new Promise(resolve => {
        const m = document.getElementById("libDiffModal");
        document.getElementById("lblDiffLib").textContent = lib;
        const body = document.getElementById("libDiffBody");
        body.innerHTML = "";
        changes.forEach((c, i) => {
            const tr = document.createElement("tr");
            tr.innerHTML = `<td>${escapeHtml(c.label)}</td>
                <td><label><input type="radio" name="libd${i}" value="mine" checked> ${escapeHtml(c.current)}</label></td>
                <td><label><input type="radio" name="libd${i}" value="lib"> ${escapeHtml(c.library)}</label></td>`;
            body.appendChild(tr);
        });
        const all = v => body.querySelectorAll(`input[value="${v}"]`).forEach(r => { r.checked = true; });
        const done = res => { m.classList.add("hidden"); ["btnDiffAllMine", "btnDiffAllLib", "btnDiffCancel", "btnDiffApply"].forEach(id => { const el = document.getElementById(id); el.replaceWith(el.cloneNode(true)); }); resolve(res); };
        document.getElementById("btnDiffAllMine").addEventListener("click", () => all("mine"));
        document.getElementById("btnDiffAllLib").addEventListener("click", () => all("lib"));
        document.getElementById("btnDiffCancel").addEventListener("click", () => done(null));
        document.getElementById("btnDiffApply").addEventListener("click", () => done(changes.filter((c, i) => body.querySelector(`input[name="libd${i}"]:checked`)?.value === "mine").map(c => c.key)));
        m.classList.remove("hidden");
    });
}

async function loadPackagesList() {
    try {
        const res = await fetch("/api/packages");
        const data = await res.json();
        pkgState.packages = data.packages || [];
        document.getElementById("countAllPkgs").textContent = pkgState.packages.length;
        renderPkgTree();
    } catch (e) {
        console.error("Fehler beim Laden der Packages:", e);
    }
}

const PKG_GROUPS = [
    { key: "SMD", label: "SMD Packages" },
    { key: "THT", label: "THT Packages" },
    { key: "Other", label: "Other Packages" },
    { key: "Fiducial", label: "Fiducials" },
    { key: "Library", label: "Library (not on board)" }
];

function renderPkgTree() {
    const list = document.getElementById("pkgTreeList");
    list.innerHTML = "";

    const query = pkgState.searchQuery.toLowerCase();
    const filtered = pkgState.packages.filter(p => {
        if (pkgState.activeCategory && (p.group || "SMD") !== pkgState.activeCategory) return false;
        if (query && !p.name.toLowerCase().includes(query)) return false;
        // Seite: Bibliotheks-Packages ohne Bauteile bleiben sichtbar
        const f = "both"; // eine Ansicht: Packages beider Seiten
        if (f !== "both" && p.sides && p.sides.top + p.sides.bot > 0 && !p.sides[f]) return false;
        return true;
    });
    pkgState.collapsedGroups = pkgState.collapsedGroups || { Library: true };

    for (const g of PKG_GROUPS) {
        const members = filtered.filter(p => (p.group || "SMD") === g.key).sort((a, b) => a.name.localeCompare(b.name));
        if (members.length === 0) continue;
        const collapsed = Boolean(pkgState.collapsedGroups[g.key]) && !query;
        const header = document.createElement("div");
        header.className = "pkg-group-header";
        const isActive = pkgState.ruleTarget?.kind === "group" && pkgState.ruleTarget.id === g.key;
        header.className = `pkg-group-header ${isActive ? "active" : ""}`;
        header.innerHTML = `<span class="caret">${collapsed ? "▸" : "▾"}</span><span class="grp-label" title="Edit group rule">${g.label}</span><span class="pkg-group-count">${members.length}</span>`;
        header.querySelector(".caret").addEventListener("click", (e) => {
            e.stopPropagation();
            pkgState.collapsedGroups[g.key] = !pkgState.collapsedGroups[g.key];
            renderPkgTree();
        });
        header.addEventListener("contextmenu", (e) => showTreeMenu(e, [[`Generate ${g.label} only`, () => generateTarget("group", g.key)]]));
        header.querySelector(".grp-label").addEventListener("click", (e) => {
            e.stopPropagation();
            selectGroupRule(g.key);
        });
        list.appendChild(header);
        if (collapsed) continue;
        for (const p of members) renderPkgTreeItem(list, p);
    }
}

function renderPkgTreeItem(list, p) {
    {
        const item = document.createElement("div");
        const inherits = p.rule?.rule_type === "inherit";
        item.className = `pkg-tree-item ${p.name === pkgState.activePkgName && pkgState.ruleTarget?.kind === "package" ? 'active' : ''} ${inherits ? 'inherits' : ''} ${p.review ? `review-${p.review}` : ""}`;
        const reviewText = { open: "Open – nothing generated yet", auto: "Auto-generated – not edited yet", done: "Edited / done" }[p.review] || "";
        item.title = `${inherits ? "Rule inherited from group" : `Own rule: ${p.rule?.rule_type}`}${reviewText ? `
${reviewText}` : ""}`;
        
        let pkgIcon = icon("package");
        if (p.category === "SMD Passives") pkgIcon = icon("shrink");
        else if (p.category === "ICs & Transistors") pkgIcon = icon("split");
        else if (p.category === "BGA & Arrays") pkgIcon = icon("target");

        item.innerHTML = `
            <div class="pkg-item-main">
                <span class="pkg-item-icon">${pkgIcon}</span>
                <span class="pkg-item-name" title="${escapeHtml(p.name)}">${escapeHtml(p.name)}</span>${inherits ? "" : '<span class="own-rule-dot">●</span>'}
                ${p.name === pkgState.activePkgName && pkgState.activePackageData?.pads?.length ? `<button class="pkg-pin-toggle" title="Expand/collapse pins">${pkgState.pinsExpanded ? "−" : "+"}</button>` : ""}
            </div>
            <div class="pkg-item-badges">
                ${p.pitch > 0 ? `<span class="pkg-badge-pitch">${p.pitch}mm</span>` : ''}
                ${p.component_count > 0 ? `<span class="pkg-badge-count" title="${p.component_count}x on board">${p.component_count}x</span>` : ''}
                ${p.sides && p.sides.bot > 0 ? `<span class="pkg-badge-side" title="TOP ${p.sides.top}× · BOT ${p.sides.bot}×">${p.sides.top}/${p.sides.bot}</span>` : ""}
            </div>
        `;

        item.addEventListener("click", async () => {
            // Package nur auf der anderen Seite: dorthin wechseln, damit Pads und Vorschau stimmen
            const s = p.sides;
            if (s && s.top + s.bot > 0 && !s[state.side || "top"]) await switchSide(s.top ? "top" : "bot", { silent: true });
            selectPackage(p.name);
        });
        item.addEventListener("contextmenu", (e) => showTreeMenu(e, [
            [`Generate ${p.name} only`, () => generateTarget("package", p.name)],
            [`Generate whole group ${(PKG_GROUPS.find(g => g.key === (p.group || "SMD")) || {}).label || p.group}`, () => generateTarget("group", p.group || "SMD")]
        ]));
        item.querySelector(".pkg-pin-toggle")?.addEventListener("click", (e) => {
            e.stopPropagation();
            pkgState.pinsExpanded = !pkgState.pinsExpanded;
            localStorage.setItem("sc.pinsOpen", pkgState.pinsExpanded ? "1" : "0");
            renderPkgTree();
        });

        list.appendChild(item);

        // Aktives Package: Pins als Unterknoten (wie PCB Investigator "1(0)", "2(1)")
        const data = pkgState.activePackageData;
        if (pkgState.pinsExpanded && p.name === pkgState.activePkgName && data && data.name === p.name && data.pads?.length) {
            const pins = document.createElement("div");
            pins.className = "pkg-pin-list";
            const ovs = data.pad_overrides || {};
            data.pads.forEach((pad, idx) => {
                const pi = document.createElement("div");
                const hasOv = ovs[pad.pin] || ovs[String(pad.pin)];
                const hidden = isPadIgnored(pad);
                pi.className = `pkg-pin-item ${isPinSelected(pad.pin) ? "active" : ""} ${hidden ? "pin-hidden" : ""}`;
                pi.innerHTML = `<span>${pad.pin}(${idx})</span><span>${pad.w}×${pad.h}${pad.drill ? ` ⌀${pad.drill}` : ""}${hidden ? " hidden" : ""}</span>${hasOv ? `<span class="pin-ov">${icon("edit")}</span>` : ""}`;
                pi.addEventListener("click", (e) => {
                    e.stopPropagation();
                    if (e.ctrlKey || e.shiftKey) {
                        const cur = pkgState.pinBarMode === "multi" ? pkgState.selectedPins : (pkgState.selectedPin !== null ? [pkgState.selectedPin] : []);
                        selectPkgPins(cur.includes(pad.pin) ? cur.filter(x => x !== pad.pin) : [...cur, pad.pin]);
                        return;
                    }
                    selectPkgPin(pad.pin, pad);
                    renderPkgTree();
                });
                pins.appendChild(pi);
            });
            sketchEntries().forEach((t, i) => {
                const si = document.createElement("div");
                si.className = `pkg-pin-item pin-sketch ${sk.sel === i ? "active" : ""}`;
                si.textContent = t;
                si.addEventListener("click", (e) => { e.stopPropagation(); sk.sel = i; renderPkgTree(); renderPkgPreview(); });
                pins.appendChild(si);
            });
            list.appendChild(pins);
        }
    }
}

// Helles Design (Standard, wie PCB-I) / dunkles Design, gespeichert im Browser
(function initTheme() {
    const apply = t => document.documentElement.setAttribute("data-theme", t);
    apply(localStorage.getItem("sc.theme") || "light");
    document.addEventListener("DOMContentLoaded", () => document.getElementById("btnTheme")?.addEventListener("click", () => {
        const t = document.documentElement.getAttribute("data-theme") === "light" ? "dark" : "light";
        localStorage.setItem("sc.theme", t);
        apply(t);
    }));
})();

// Selbstprüfung: nur Package-Namen und Abweichungen (mm), lokal – für sensible Projekte weitergebbar
async function checkPreviewAgainstBoard() {
    const d = await (await fetch("/api/check/preview")).json();
    const bad = (d.items || []).filter(x => x.deviation_mm > 0.05 || x.origin_offset_mm > 0.05);
    const lines = bad.slice(0, 30).map(x => `${x.package} (${x.side.toUpperCase()}): shape ${x.deviation_mm.toFixed(3)} mm · origin offset ${x.origin_offset_mm.toFixed(3)} mm · pads editor/board ${x.pads}/${x.board_pads}`);
    const text = bad.length ? `${bad.length} of ${(d.items || []).length} packages differ:\n${lines.join("\n")}` : `All ${(d.items || []).length} packages match the board (≤ 0.05 mm).`;
    console.log(text);
    alert(text);
}
document.addEventListener("DOMContentLoaded", () => document.getElementById("btnCheckPreview")?.addEventListener("click", checkPreviewAgainstBoard));

// -------------------------------------------------------------
// Menüleiste: jeder Eintrag löst den vorhandenen Knopf aus (id) oder ruft eine Funktion auf (fn).
// Einträge ohne Ziel in diesem Fenster entfallen.
// -------------------------------------------------------------
const clickEl = sel => () => document.querySelector(sel)?.click();
const inGenerator = fn => async () => {
    if (currentWorkspaceMode === "board" && !document.body.classList.contains("view-package-only")) document.getElementById("btnViewPackage")?.click();
    await new Promise(r => setTimeout(r, 150));
    fn();
};
const APP_MENUS = [
    ["File", [
        ["Open Dataset…", "#btnOpenDataset", "Ctrl+O"],
        ["Open Project…", '#projectMenuList [data-act="open"]'],
        ["Save Project", '#projectMenuList [data-act="save"]', "Ctrl+S"],
        ["Save Project As…", '#projectMenuList [data-act="saveas"]', "Ctrl+Shift+S"],
        "-",
        ["Load Library…", null, null, inGenerator(() => { document.getElementById("btnLibLoad")?.click(); })],
        ["Save Library", "#btnSaveCurrentLib"],
        ["Import PCB-I Rules…", "#btnPcbiImport"],
        ["Export PCB-I Rules…", "#btnPcbiExport"],
        "-",
        ["Gerber Export…", "#btnExport", "Ctrl+E"],
        "-",
        ["Close Dataset", "#btnCloseDataset"],
    ]],
    ["Edit", [
        ["Undo", "#btnUndo", "Ctrl+Z"],
        ["Redo", "#btnRedo", "Ctrl+Y"],
        ["History…", "#btnHistory"],
        "-",
        ["Add Aperture", "#btnAddOpening"],
        ["Draw Step Zone", "#btnNewZone"],
        ["Measure", null, "M", () => toggleMeasureTool()],
    ]],
    ["Extras", [
        ["Auto Classification", null, null, inGenerator(clickEl("#btnAutoClassify"))],
        ["Missing Paste from Copper…", null, null, inGenerator(clickEl("#btnAddMissingPaste"))],
        ["Check Preview ↔ Board", "#btnCheckPreview"],
        "-",
        ["Web Check", "#btnCheckBridges"],
        ["Stencil Analysis", "#btnAnalysis"],
        ["IPC Limits…", "#btnIpcProfiles"],
        "-",
        ["Pin-in-Paste…", "#btnOpenPipModal"],
        ["Release Calculator…", "#btnOpenCalculator"],
    ]],
    ["View", [
        ["Board View", "#btnViewBoard"],
        ["Stencil Generator", "#btnViewPackage"],
        ["Stencil Generator in Separate Window", "#btnDetachGenerator"],
        "-",
        ["Toggle Light / Dark", "#btnTheme"],
        ["Smaller Font", null, null, () => stepUiScale(-1)],
        ["Larger Font", null, null, () => stepUiScale(1)],
    ]],
    ["Help", [
        ["Keyboard Shortcuts", null, null, () => alert("Ctrl+O  Open Dataset\nCtrl+S  Save Project\nCtrl+E  Gerber Export\nCtrl+Z / Ctrl+Y  Undo / Redo\nM  Measure\nArrow Up/Down  Switch package (Stencil Generator)\nRight mouse drag  Pan view\nCtrl+A, Del, R, Ctrl+D  Apertures on board")],
    ]],
];

function stepUiScale(dir) {
    const sel = document.getElementById("selectUiScale");
    if (!sel) return;
    const i = Math.min(sel.options.length - 1, Math.max(1, sel.selectedIndex + dir));
    sel.selectedIndex = i;
    sel.dispatchEvent(new Event("change", { bubbles: true }));
}

function buildMenuBar() {
    const bar = document.getElementById("appMenuBar");
    if (!bar) return;
    bar.innerHTML = "";
    const closeAll = () => bar.querySelectorAll(".app-menu.open").forEach(m => m.classList.remove("open"));
    for (const [title, items] of APP_MENUS) {
        const menu = document.createElement("div");
        menu.className = "app-menu";
        const head = document.createElement("button");
        head.className = "app-menu-head";
        head.textContent = title;
        const list = document.createElement("div");
        list.className = "app-menu-list";
        for (const it of items) {
            if (it === "-") { list.insertAdjacentHTML("beforeend", '<div class="app-menu-sep"></div>'); continue; }
            const [label, sel, key, fn] = it;
            if (sel && !document.querySelector(sel)) continue;
            const b = document.createElement("button");
            b.innerHTML = `<span>${escapeHtml(label)}</span>${key ? `<kbd>${escapeHtml(key)}</kbd>` : ""}`;
            b.addEventListener("click", (e) => {
                e.stopPropagation();
                closeAll();
                const target = sel && document.querySelector(sel);
                if (target?.disabled) return;
                fn ? fn() : target?.click();
            });
            list.appendChild(b);
        }
        list.querySelectorAll(".app-menu-sep + .app-menu-sep").forEach(s => s.remove());
        head.addEventListener("click", (e) => {
            e.stopPropagation();
            const was = menu.classList.contains("open");
            closeAll();
            if (!was) menu.classList.add("open");
        });
        // Bei offenem Menü wechselt Überfahren das Menü (wie in Windows)
        head.addEventListener("mouseenter", () => { if (bar.querySelector(".app-menu.open") && !menu.classList.contains("open")) { closeAll(); menu.classList.add("open"); } });
        menu.append(head, list);
        bar.appendChild(menu);
    }
    document.addEventListener("click", closeAll);
    document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeAll(); });
}
document.addEventListener("DOMContentLoaded", buildMenuBar);

// Nach Generate: Platine, Lagen, Baum, Vorschau und Verlauf auffrischen
async function afterGenerate() {
    await refreshApertureData();
    await loadPackagesList();
    if (pkgState.activePkgName) await selectPackage(pkgState.activePkgName);
    refreshHistory();
}

// Nur ein Ziel generieren (Rechtsklick im Baum): Package oder Gruppe
async function generateTarget(kind, id) {
    const isActive = kind === "package" && id === pkgState.activePkgName && pkgState.ruleTarget?.kind === "package";
    const res = await fetch(`/api/${kind}/${encodeURIComponent(id)}/generate`, { method: "POST", headers: { "Content-Type": "application/json" }, body: isActive ? JSON.stringify(collectRuleFromInputs()) : "{}" });
    const data = await res.json().catch(() => ({}));
    if (!res.ok || data.status === "error") { showToast(`Generate failed (${id})`, 5000); return; }
    await afterGenerate();
    showToast(`${data.modified_count ?? 0} pads generated: ${id}`);
}

function showTreeMenu(e, items) {
    e.preventDefault();
    document.getElementById("treeCtxMenu")?.remove();
    const m = document.createElement("div");
    m.id = "treeCtxMenu";
    m.className = "tree-ctx-menu";
    for (const [label, fn] of items) {
        const btn = document.createElement("button");
        btn.textContent = label;
        btn.addEventListener("click", () => { m.remove(); fn(); });
        m.appendChild(btn);
    }
    m.style.left = `${e.clientX}px`;
    m.style.top = `${e.clientY}px`;
    document.body.appendChild(m);
    setTimeout(() => document.addEventListener("click", () => m.remove(), { once: true }), 0);
}

// -------------------------------------------------------------
// Freihand im Stencil Generator: Pads ausblenden (Rechtsklick) und eigene Öffnungen skizzieren.
// Gespeichert im Package (Bibliothek), "Generate" überträgt es auf alle Bauteile des Packages.
// -------------------------------------------------------------
const sk = { tool: null, start: null, cur: null, pts: [], sel: -1, drag: null };

function pkgFreehand() {
    const f = pkgState.activePackageData?.freehand || {};
    return { ignored: (f.ignored || []).map(p => [...p]), shapes: (f.shapes || []).map(s => s.map(p => [...p])) };
}

async function saveFreehand(fh) {
    const name = pkgState.activePkgName;
    if (!name || !pkgState.activePackageData) return;
    pkgState.activePackageData.freehand = fh;
    renderPkgPreview();
    const res = await fetch(`/api/package/${encodeURIComponent(name)}/freehand`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(fh) });
    const data = await res.json();
    if (data.preview && pkgState.activePkgName === name) {
        pkgState.activePackageData = data.preview;
        populatePinDropdown(data.preview.pads, data.preview.pad_overrides);
        renderPkgTree();
        renderPkgPreview();
    }
    noteRuleEdit();
    loadPackagesList();
}

// Kleinstes Pad unter dem Punkt (Finger liegen auf großen Pads)
function pkgPadAt(x, y) {
    let best = null;
    for (const p of pkgState.activePackageData?.pads || []) {
        if (Math.abs(x - p.cx) <= p.w / 2 && Math.abs(y - p.cy) <= p.h / 2 && (!best || p.w * p.h < best.w * best.h)) best = p;
    }
    return best;
}
const isPadIgnored = (p) => pkgFreehand().ignored.some(q => Math.hypot(q[0] - p.cx, q[1] - p.cy) < 0.01);

function pointInPoly(x, y, pts) {
    let inside = false;
    for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
        const [xi, yi] = pts[i], [xj, yj] = pts[j];
        if ((yi > y) !== (yj > y) && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
    }
    return inside;
}
function sketchAt(x, y) {
    const shapes = pkgFreehand().shapes;
    for (let i = shapes.length - 1; i >= 0; i--) if (pointInPoly(x, y, shapes[i])) return i;
    return -1;
}

// Fangen: Ecke vor Kantenmitte vor Kante (Lotfußpunkt), ca. 10 px Reichweite; sonst 0,01-mm-Raster; Umschalt = frei
function sketchOutlines() {
    const data = pkgState.activePackageData || {};
    const rect = (cx, cy, w, h) => [[cx - w / 2, cy - h / 2], [cx + w / 2, cy - h / 2], [cx + w / 2, cy + h / 2], [cx - w / 2, cy + h / 2]];
    const out = [];
    for (const p of data.pads || []) {
        out.push({ pts: rect(p.cx, p.cy, p.w, p.h), label: `Pad ${p.pin}` });
        if (p.mask_w && p.mask_h) out.push({ pts: rect(p.cx, p.cy, p.mask_w, p.mask_h), label: `Mask ${p.pin}` });
    }
    for (const s of data.paste || []) if (s.type !== "freehand") out.push({ pts: s.pts && s.pts.length <= 64 ? s.pts : rect(s.cx, s.cy, s.w, s.h), label: `Paste ${s.pin ?? ""}` });
    pkgFreehand().shapes.forEach((s, i) => { if (!(sk.drag && sk.sel === i)) out.push({ pts: s, label: `Sketch ${i + 1}` }); });
    return out;
}
function sketchSnapPoint(x, y) {
    const reach = 10 / pkgState.zoom;
    let best = null;
    const take = (px, py, kind, label, prio) => {
        const d = Math.hypot(px - x, py - y);
        if (d > reach) return;
        if (!best || prio < best.prio || (prio === best.prio && d < best.d)) best = { x: px, y: py, kind, label, prio, d };
    };
    for (const o of sketchOutlines()) {
        const n = o.pts.length;
        for (let i = 0; i < n; i++) {
            const [ax, ay] = o.pts[i], [bx, by] = o.pts[(i + 1) % n];
            take(ax, ay, "corner", o.label, 0);
            take((ax + bx) / 2, (ay + by) / 2, "mid", o.label, 1);
            const vx = bx - ax, vy = by - ay, L = vx * vx + vy * vy;
            if (L > 1e-12) {
                const t = Math.max(0, Math.min(1, ((x - ax) * vx + (y - ay) * vy) / L));
                take(ax + t * vx, ay + t * vy, "edge", o.label, 2);
            }
        }
    }
    return best;
}
function sketchSnap(x, y, free) {
    sk.snap = free ? null : sketchSnapPoint(x, y);
    if (sk.snap) return [sk.snap.x, sk.snap.y];
    const g = (v) => Math.round(v * 100) / 100;
    return free ? [x, y] : [g(x), g(y)];
}

function setSketchTool(tool) {
    sk.tool = sk.tool === tool ? null : tool;
    sk.start = sk.cur = null;
    sk.pts = [];
    if (sk.tool && measureState.active) toggleMeasureTool(false);
    document.getElementById("btnSketchRect")?.classList.toggle("active", sk.tool === "rect");
    document.getElementById("btnSketchPoly")?.classList.toggle("active", sk.tool === "poly");
    if (pkgCanvas) pkgCanvas.style.cursor = sk.tool ? "crosshair" : "";
    renderPkgPreview();
}

function finishPoly() {
    const pts = sk.pts.filter((p, i, a) => i === 0 || Math.hypot(p[0] - a[i - 1][0], p[1] - a[i - 1][1]) > 1e-4);
    sk.pts = [];
    if (pts.length < 3) { renderPkgPreview(); return; }
    const fh = pkgFreehand();
    fh.shapes.push(pts);
    sk.sel = fh.shapes.length - 1;
    saveFreehand(fh);
}

// true = Klick gehört dem Freihand-Werkzeug
function sketchMouseDown(e, x, y) {
    if (e.button !== 0) return false;
    const [sx, sy] = sketchSnap(x, y, e.shiftKey);
    if (sk.tool === "rect") {
        sk.start = sk.cur = [sx, sy];
        return true;
    }
    if (sk.tool === "poly") {
        const first = sk.pts[0];
        if (first && sk.pts.length >= 3 && Math.hypot(first[0] - sx, first[1] - sy) < 6 / pkgState.zoom) finishPoly();
        else { sk.pts.push([sx, sy]); renderPkgPreview(); }
        return true;
    }
    const shapes = pkgFreehand().shapes;
    if (sk.sel >= 0 && shapes[sk.sel]) {
        const vi = shapes[sk.sel].findIndex(p => Math.hypot(p[0] - x, p[1] - y) < 6 / pkgState.zoom);
        if (vi >= 0) { sk.drag = { kind: "vertex", i: vi, orig: shapes[sk.sel].map(p => [...p]), moved: false }; return true; }
    }
    const hit = sketchAt(x, y);
    if (hit >= 0) {
        sk.sel = hit;
        sk.drag = { kind: "move", from: [sx, sy], orig: shapes[hit].map(p => [...p]), moved: false };
        renderPkgPreview();
        return true;
    }
    if (sk.sel >= 0) { sk.sel = -1; renderPkgPreview(); }
    return false;
}

function sketchMouseMove(e) {
    if (!sk.tool && !sk.drag) return false;
    const { x, y } = pkgScreenToWorld(e.clientX, e.clientY);
    const [sx, sy] = sketchSnap(x, y, e.shiftKey);
    sk.cur = [sx, sy];
    const d = sk.drag;
    if (d) {
        const fh = pkgState.activePackageData.freehand;
        let s = d.orig.map(p => [...p]);
        if (d.kind === "move") {
            s = s.map(p => [p[0] + sx - d.from[0], p[1] + sy - d.from[1]]);
        } else {
            // Ecke ziehen; achsparalleles Rechteck bleibt Rechteck
            const [ox, oy] = d.orig[d.i];
            if (s.length === 4) for (const p of s) { if (p[0] === ox) p[0] = sx; if (p[1] === oy) p[1] = sy; }
            s[d.i] = [sx, sy];
        }
        fh.shapes[sk.sel] = s;
        d.moved = true;
    }
    renderPkgPreview();
    return true;
}

function sketchMouseUp() {
    if (sk.tool === "rect" && sk.start && sk.cur) {
        const [x0, y0] = sk.start, [x1, y1] = sk.cur;
        sk.start = null;
        if (Math.abs(x1 - x0) > 0.02 && Math.abs(y1 - y0) > 0.02) {
            const fh = pkgFreehand();
            fh.shapes.push([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
            sk.sel = fh.shapes.length - 1;
            saveFreehand(fh);
        } else renderPkgPreview();
        return true;
    }
    if (sk.drag) {
        const moved = sk.drag.moved;
        sk.drag = null;
        if (moved) saveFreehand(pkgFreehand());
        return true;
    }
    return false;
}

function sketchKey(e) {
    if (currentWorkspaceMode !== "package") return;
    const tag = (document.activeElement?.tagName || "").toLowerCase();
    if (tag === "input" || tag === "select" || tag === "textarea") return;
    if (e.key === "Escape" && (sk.tool || sk.sel >= 0)) {
        if (sk.pts.length || sk.start) { sk.pts = []; sk.start = null; renderPkgPreview(); }
        else if (sk.tool) setSketchTool(sk.tool);
        else { sk.sel = -1; renderPkgPreview(); }
    } else if (e.key === "Enter" && sk.tool === "poly") {
        finishPoly();
    } else if ((e.key === "Delete" || e.key === "Backspace") && sk.sel >= 0) {
        const fh = pkgFreehand();
        fh.shapes.splice(sk.sel, 1);
        sk.sel = -1;
        saveFreehand(fh);
        e.preventDefault();
    }
}

function sketchContextMenu(e) {
    const { x, y } = pkgScreenToWorld(e.clientX, e.clientY);
    const items = [];
    const hit = sketchAt(x, y);
    if (hit >= 0) items.push(["Delete Sketch", () => { const fh = pkgFreehand(); fh.shapes.splice(hit, 1); sk.sel = -1; saveFreehand(fh); }]);
    const pad = pkgPadAt(x, y);
    if (pad) {
        const off = isPadIgnored(pad);
        items.push([off ? `Pad (pin ${pad.pin}) with paste again` : `Hide pad (pin ${pad.pin}) – no paste`, () => {
            const fh = pkgFreehand();
            fh.ignored = off ? fh.ignored.filter(q => Math.hypot(q[0] - pad.cx, q[1] - pad.cy) >= 0.01) : [...fh.ignored, [pad.cx, pad.cy]];
            saveFreehand(fh);
        }]);
    }
    items.push(["Sketch Rectangle", () => setSketchTool("rect")], ["Sketch Polygon", () => setSketchTool("poly")]);
    const fh = pkgFreehand();
    if (fh.ignored.length || fh.shapes.length) items.push([`Reset Freehand (${fh.ignored.length} off, ${fh.shapes.length} sketch(es))`, () => { sk.sel = -1; saveFreehand({ ignored: [], shapes: [] }); }]);
    showTreeMenu(e, items);
}

// Zeichnen im Package-Rahmen (innerhalb der Vorschau-Transformation)
function drawFreehandOverlay(ctx, data, zoom) {
    const fh = data.freehand || {};
    ctx.lineWidth = 1.5 / zoom;
    ctx.strokeStyle = "#ef4444";
    ctx.setLineDash([4 / zoom, 3 / zoom]);
    for (const p of data.pads || []) {
        if (!(fh.ignored || []).some(q => Math.hypot(q[0] - p.cx, q[1] - p.cy) < 0.01)) continue;
        const [x0, y0, x1, y1] = [p.cx - p.w / 2, p.cy - p.h / 2, p.cx + p.w / 2, p.cy + p.h / 2];
        ctx.strokeRect(x0, y0, p.w, p.h);
        ctx.beginPath(); ctx.moveTo(x0, y0); ctx.lineTo(x1, y1); ctx.moveTo(x0, y1); ctx.lineTo(x1, y0); ctx.stroke();
    }
    ctx.setLineDash([]);
    const sel = (fh.shapes || [])[sk.sel];
    if (sel) {
        ctx.strokeStyle = "#facc15";
        ctx.beginPath(); sel.forEach((p, i) => i ? ctx.lineTo(p[0], p[1]) : ctx.moveTo(p[0], p[1])); ctx.closePath(); ctx.stroke();
        ctx.fillStyle = "#facc15";
        const hs = 4 / zoom;
        for (const p of sel) ctx.fillRect(p[0] - hs, p[1] - hs, hs * 2, hs * 2);
    }
    ctx.strokeStyle = "#22d3ee";
    ctx.setLineDash([5 / zoom, 3 / zoom]);
    if (sk.tool === "rect" && sk.start && sk.cur) {
        ctx.strokeRect(Math.min(sk.start[0], sk.cur[0]), Math.min(sk.start[1], sk.cur[1]), Math.abs(sk.cur[0] - sk.start[0]), Math.abs(sk.cur[1] - sk.start[1]));
    } else if (sk.tool === "poly" && sk.pts.length) {
        ctx.beginPath();
        sk.pts.forEach((p, i) => i ? ctx.lineTo(p[0], p[1]) : ctx.moveTo(p[0], p[1]));
        if (sk.cur) ctx.lineTo(sk.cur[0], sk.cur[1]);
        ctx.stroke();
    }
    ctx.setLineDash([]);
}

// Fangmarke: Quadrat = Ecke, Dreieck = Kantenmitte, Kreuz = Kante
function drawSnapMarker(ctx) {
    if (!sk.snap || !(sk.tool || sk.drag)) return;
    const p = pkgWorldToScreen(sk.snap.x, sk.snap.y);
    ctx.save();
    resetToCssPixels(ctx);
    ctx.strokeStyle = "#22d3ee";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    if (sk.snap.kind === "corner") ctx.rect(p.x - 5, p.y - 5, 10, 10);
    else if (sk.snap.kind === "mid") { ctx.moveTo(p.x, p.y - 6); ctx.lineTo(p.x + 6, p.y + 5); ctx.lineTo(p.x - 6, p.y + 5); ctx.closePath(); }
    else { ctx.moveTo(p.x - 5, p.y - 5); ctx.lineTo(p.x + 5, p.y + 5); ctx.moveTo(p.x + 5, p.y - 5); ctx.lineTo(p.x - 5, p.y + 5); }
    ctx.stroke();
    ctx.font = "11px sans-serif";
    ctx.fillStyle = "#22d3ee";
    ctx.fillText(`${{ corner: "Corner", mid: "Midpoint", edge: "Edge" }[sk.snap.kind]} ${sk.snap.label}`, p.x + 9, p.y + 14);
    ctx.restore();
}

// Maße beim Aufziehen (Bildschirm-Pixel)
function drawSketchLabel(ctx) {
    drawSnapMarker(ctx);
    if (!sk.cur || !((sk.tool === "rect" && sk.start) || (sk.tool === "poly" && sk.pts.length))) return;
    const a = sk.tool === "rect" ? sk.start : sk.pts[sk.pts.length - 1];
    const txt = sk.tool === "rect" ? `${Math.abs(sk.cur[0] - a[0]).toFixed(3)} × ${Math.abs(sk.cur[1] - a[1]).toFixed(3)} mm` : `${Math.hypot(sk.cur[0] - a[0], sk.cur[1] - a[1]).toFixed(3)} mm`;
    const sp = pkgWorldToScreen(sk.cur[0], sk.cur[1]);
    ctx.save();
    resetToCssPixels(ctx);
    ctx.font = "12px sans-serif";
    ctx.fillStyle = "#22d3ee";
    ctx.fillText(txt, sp.x + 12, sp.y - 10);
    ctx.restore();
}

document.addEventListener("DOMContentLoaded", () => {
    document.getElementById("btnSketchRect")?.addEventListener("click", () => setSketchTool("rect"));
    document.getElementById("btnSketchPoly")?.addEventListener("click", () => setSketchTool("poly"));
    window.addEventListener("keydown", sketchKey);
});

// Platinenumgebung der ersten Instanz (Kupfer + Schablone der Nachbarn) für die Vorschau
async function loadPkgBoardContext() {
    const name = pkgState.activePkgName;
    pkgState.boardContext = null;
    if (!name) { renderPkgPreview(); return; }
    const r = document.getElementById("chkPrevBoard")?.checked ? (document.getElementById("selPrevBoardRadius")?.value || 4) : 0.5;
    try {
        const res = await fetch(`/api/package/${encodeURIComponent(name)}/context?radius=${r}`);
        const data = await res.json();
        if (name === pkgState.activePkgName) {
            if (!document.getElementById("chkPrevBoard")?.checked) data.items = [];
            const turned = Math.abs(pkgInstanceTf().s) > 0.7;
            pkgState.boardContext = data;
            if (turned !== Math.abs(pkgInstanceTf().s) > 0.7) autoFitPackagePreview(pkgState.activePackageData);
        }
    } catch (e) {
        console.error("Platinenumgebung:", e);
    }
    renderPkgPreview();
}

function drawPkgBoardContext(ctx, zoom) {
    const items = pkgState.boardContext?.items;
    if (!items?.length) return;
    const poly = pts => { ctx.beginPath(); pts.forEach(([x, y], i) => i ? ctx.lineTo(x, y) : ctx.moveTo(x, y)); ctx.closePath(); };
    ctx.lineWidth = 1 / zoom;
    // Kundenpaste der gezeigten Instanz (rosa Umriss wie in der Hauptansicht)
    if (document.getElementById("chkPrevCustomer")?.checked !== false) {
        ctx.strokeStyle = "rgba(236, 72, 153, 0.95)";
        ctx.fillStyle = "rgba(236, 72, 153, 0.18)";
        ctx.setLineDash([4 / zoom, 3 / zoom]);
        for (const it of items) if (it.own && it.customer) { poly(it.customer); ctx.fill(); ctx.stroke(); }
        ctx.setLineDash([]);
    }
    for (const it of items) {
        if (it.own) continue;
        if (pkgState.showCopper !== false && it.copper) {
            ctx.fillStyle = "rgba(0, 168, 140, 0.55)";
            poly(it.copper);
            ctx.fill();
        }
        if (pkgState.showPaste !== false) {
            ctx.strokeStyle = "rgba(148, 163, 184, 0.9)";
            ctx.fillStyle = "rgba(148, 163, 184, 0.25)";
            for (const st of it.stencil || []) { poly(st); ctx.fill(); ctx.stroke(); }
        }
    }
}

async function selectPackage(pkgName) {
    // Eine Ansicht für beide Seiten: liegt das Package nur auf der anderen Seite, still dorthin wechseln
    // (sonst fehlen Lage, BOT-Spiegelung und Umgebung im Editor)
    const sd = (pkgState.packages || []).find(x => x.name === pkgName)?.sides;
    if (sd && sd.top + sd.bot > 0 && !sd[state.side || "top"]) await switchSide(sd.top ? "top" : "bot", { silent: true });
    pkgState.activePkgName = pkgName;
    pkgState.selectedPin = null;
    document.getElementById("singlePadBar").classList.add("hidden");

    // Highlight in list and scroll into view
    document.querySelectorAll(".pkg-tree-item").forEach(el => {
        const isMatch = el.querySelector(".pkg-item-name")?.textContent === pkgName;
        el.classList.toggle("active", isMatch);
        if (isMatch) {
            el.scrollIntoView({ block: "nearest", behavior: "smooth" });
        }
    });

    try {
        const res = await fetch(`/api/package/${encodeURIComponent(pkgName)}`);
        const data = await res.json();
        pkgState.activePackageData = data;
        state.highlightPkg = (state.apertures || []).some(a => a.pkg_name === pkgName) ? pkgName : null;
        state.highlightPin = null;
        window.scBroadcast?.("pkg-focus", { pkg: pkgName, pin: null });

        // UI Header aktualisieren
        document.getElementById("lblActivePkgName").textContent = data.name;
        document.getElementById("lblActivePkgPins").textContent = data.pin_count;
        document.getElementById("lblActivePkgPitch").textContent = data.pitch > 0 ? `${data.pitch} mm` : "N/A";
        document.getElementById("lblActivePkgBoardCount").textContent = `${data.component_count}x`;

        document.getElementById("canvasPkgTitle").textContent = data.name;
        document.getElementById("canvasPkgPitch").textContent = data.pitch > 0 ? `Pitch: ${data.pitch} mm` : "";

        // Pin Dropdown befüllen
        populatePinDropdown(data.pads, data.pad_overrides);
        loadPkgBoardContext();

        // Regeleingaben aktualisieren (bei Inherit: Werte der geerbten Regel zeigen)
        pkgState.ruleTarget = { kind: "package", id: data.name };
        pkgState.effectiveRuleType = data.effective_rule?.rule_type;
        populateRuleInputs(data.rule?.rule_type === "inherit" ? { ...data.effective_rule, rule_type: "inherit" } : data.rule);
        updateRuleSourceBanner();
        document.querySelectorAll(".pkg-group-header").forEach(el => el.classList.remove("active"));

        // Auto-Fit Zoom für Package
        autoFitPackagePreview(data);
        openAllPinsBar(); // Pin-Editor standardmäßig für alle Pins
        renderPkgTree(); // Pins des aktiven Packages im Baum anzeigen
    } catch (e) {
        console.error("Fehler beim Abrufen der Package-Vorschau:", e);
    }
}

function populatePinDropdown(pads, overrides = {}) {
    const select = document.getElementById("selectPinDropdown");
    if (!select) return;
    select.innerHTML = `<option value="all">All Pins (whole package)</option>`;
    if (!pads || pads.length === 0) return;
    for (const p of pads) {
        const opt = document.createElement("option");
        opt.value = p.pin;
        const hasOv = overrides && (overrides[p.pin] || overrides[String(p.pin)]);
        const hidden = isPadIgnored(p);
        opt.textContent = `Pin ${p.pin} (${p.w}x${p.h}mm)${hasOv ? " (modified)" : ""}${hidden ? " – hidden" : ""}`;
        if (hidden) opt.style.color = "#dc2626";
        select.appendChild(opt);
    }
    sketchEntries().forEach((t, i) => {
        const opt = document.createElement("option");
        opt.value = `sketch:${i}`;
        opt.textContent = t;
        opt.style.color = "#d97706";
        select.appendChild(opt);
    });
}

// "Sketch 1 (0.30x0.20mm) → Pin 4" je Skizze (Pin = nächstes Pad, wie in der Engine)
function sketchEntries() {
    const data = pkgState.activePackageData;
    return pkgFreehand().shapes.map((s, i) => {
        const xs = s.map(p => p[0]), ys = s.map(p => p[1]);
        const w = Math.max(...xs) - Math.min(...xs), h = Math.max(...ys) - Math.min(...ys);
        const cx = xs.reduce((a, b) => a + b, 0) / xs.length, cy = ys.reduce((a, b) => a + b, 0) / ys.length;
        let near = null;
        for (const p of data?.pads || []) if (!near || Math.hypot(p.cx - cx, p.cy - cy) < Math.hypot(near.cx - cx, near.cy - cy)) near = p;
        return `Sketch ${i + 1} (${w.toFixed(2)}x${h.toFixed(2)}mm)${near ? ` → Pin ${near.pin}` : ""}`;
    });
}

function autoFitPackagePreview(data) {
    if (!pkgContainer) return;
    const rect = pkgContainer.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;

    // Bounding Box der Pads ermitteln
    let minX = -1, maxX = 1, minY = -1, maxY = 1;
    if (data.pads && data.pads.length > 0) {
        minX = Math.min(...data.pads.map(p => p.cx - p.w/2));
        maxX = Math.max(...data.pads.map(p => p.cx + p.w/2));
        minY = Math.min(...data.pads.map(p => p.cy - p.h/2));
        maxY = Math.max(...data.pads.map(p => p.cy + p.h/2));
    }
    let spanX = Math.max(2.0, (maxX - minX) * 1.6);
    let spanY = Math.max(2.0, (maxY - minY) * 1.6);
    if (Math.abs(pkgInstanceTf().s) > 0.7) [spanX, spanY] = [spanY, spanX];

    pkgState.zoom = Math.min(w / spanX, h / spanY, 150);
    pkgState.panX = 0;
    pkgState.panY = 0;
}

function populateRuleInputs(rule) {
    if (!rule) return;
    document.getElementById("selectPkgRule").value = rule.rule_type || "simple_rect";
    document.getElementById("propCornerRadius").value = rule.corner_radius ?? 0.0;
    document.getElementById("propUndersize").value = rule.undersize_w ?? 0.025;
    document.getElementById("propHouseCut").value = rule.house_cut_pct ?? 35;
    document.getElementById("propPitchRed").value = rule.pitch_reduction_pct ?? 20;
    document.getElementById("propLengthUse").value = rule.length_usage_pct ?? 95;
    document.getElementById("propGapRatio").value = rule.gap_ratio ?? 1.0;
    document.getElementById("propLengthComp").value = rule.length_comp_pct ?? 70;
    document.getElementById("propOutwardShift").value = rule.outward_shift ?? 0.025;
    document.getElementById("propOffsetMoveAway").value = rule.offset_move_away ?? 0.0;
    document.getElementById("propMoveAxis").value = rule.move_axis || "long";
    document.getElementById("propOffsetX").value = rule.offset_x ?? 0.0;
    document.getElementById("propOffsetY").value = rule.offset_y ?? 0.0;
    document.getElementById("propBridgeX").value = rule.bridge_count_x ?? 0;
    document.getElementById("propBridgeY").value = rule.bridge_count_y ?? 0;
    document.getElementById("propBridgeWidth").value = rule.bridge_width ?? 0.30;
    document.getElementById("propRotate45").checked = Boolean(rule.rotate_bridges_45);
    document.getElementById("propTryDirection").checked = Boolean(rule.try_find_direction);
    const thEl = document.getElementById("propToeHeelShift");
    if (thEl) thEl.value = rule.toe_heel_shift ?? 0.0;
    const csEl = document.getElementById("propChamferSize");
    if (csEl) csEl.value = rule.chamfer_size ?? 0.0;
    const cmEl = document.getElementById("propChamferMode");
    if (cmEl) cmEl.value = rule.chamfer_mode || "all";
    const pmmEl = document.getElementById("propPinMinusMask");
    if (pmmEl) pmmEl.checked = Boolean(rule.pin_minus_mask);
    const setv = (id, v) => { const el = document.getElementById(id); if (el) el.value = v; };
    setv("propSizeW", rule.size_w ?? 0);
    setv("propSizeH", rule.size_h ?? 0);
    setv("propDonutInner", rule.donut_inner_dia ?? 0);
    setv("propBgaThreshold", rule.bga_threshold_square ?? 0.41);
    setv("propChipThreshold", rule.chip_threshold_houses ?? 2.49);
    setv("propInvertHouse", rule.invert_house_below ?? 0.75);
    setv("propMiniChip", rule.mini_chip_reduction ?? 0.075);
    setv("propMinArrayField", rule.min_array_field ?? 0.75);
    setv("propArrayDistance", rule.distance_array_fields ?? 0.25);
    setv("propUtilArray", rule.utilization_array_pct ?? 40);
    setv("propUtilHouse", rule.utilization_house_pct ?? 0);
    setv("propPipPcb", rule.pip_pcb_thickness ?? 1.6);
    setv("propPipMetal", rule.pip_metal_pct ?? 50);
    setv("propPipClearance", rule.pip_pin_clearance ?? 0.25);
    setv("propPipShape", rule.pip_shape || "circle");
    setv("propPipMax", rule.pip_max_size ?? 0);
    setv("propArraySizeW", rule.array_size_w ?? 0);
    setv("propArraySizeH", rule.array_size_h ?? 0);
    setv("propArrayCountX", rule.array_count_x ?? 2);
    setv("propArrayCountY", rule.array_count_y ?? 2);
    setv("propDonutBridges", rule.donut_bridges ?? 4);
    setv("propDonutRotation", rule.donut_rotation ?? 0);
    setv("propDonutInnerPct", rule.donut_inner_pct ?? 50);
    setv("propDonutOuterPct", rule.donut_outer_pct ?? 100);
    setv("propMaxElement", rule.max_element_size ?? 5);
    setv("propStegBig", rule.steg_big_elements ?? 0.75);
    const chk = (id, v) => { const el = document.getElementById(id); if (el) el.checked = Boolean(v); };
    chk("propArrayRound", rule.array_round);
    chk("propIgnoreMask", rule.ignore_mask);
    updateRuleFieldVisibility();
}

// Beschreibung der Regeln (wie im PCB Investigator unter der Parameterliste)
const RULE_DESCRIPTIONS = {
    package_group: "Chooses the aperture per pad automatically: BGA pads 1:1, large areas as array fields (min. field size, field distance, area utilization), 2-pin chips as HouseBuilder (small ones as Inverted House), otherwise rectangle with undersize and corner radius.",
    simple_rect: "Create one surface with fix size and offset away from the center. Optional with webs (Bridge Count) and fixed size.",
    house_builder: "House shape: tip toward component body, prevents tombstoning and solder balls on chip components.",
    inverted_house_builder: "Create pads in form of small inverted houses. Tip outward; apertures above Max Size of one Element are split.",
    elongated_pitch: "Elongated & narrow: narrows across the pitch (paste gap = factor × aperture width), lengthens in proportion and shifts the aperture outward.",
    array_slicing: "Splits the area into fields with webs (Bridge Count / Width), optionally rotated 45°.",
    donut: "Create donut for each pin pad. Ring with Count Bridges webs (Bridge Width = Size Bridge), outer Ø in % of pad size, inner Ø in % (or fixed / drill + 0.1 mm), rotatable.",
    array_pads: "Create array pads, they are cutted by mask opening. Fields from min. field size, field distance and utilization, or with fixed field size.",
    array_fix: "Create array pads, with fix count in x and y direction. Fields that are too small (below Minimum Array Field Size) reduce the count.",
    undersize: "Create undersize for each pin pad. Apertures above Max Size of one Element are split with Steg for Big Elements.",
    pin_mask: "Use the component pins and mask to create free areas as stencil pads: pad ∩ mask opening, then undersize.",
    take_existing: "Takes the paste apertures unchanged from the customer dataset.",
    inherit: "Inherit: The package uses the rule of its group (SMD/THT/Other/Fiducials). Changing a parameter creates an own package rule.",
    pin_in_paste: "Pin-in-Paste: aperture per THT pin from drill (ODB++), board thickness, metal content and stencil thickness (IPC-7525, +20 % meniscus); limited by max. size or pitch − min. web.",
    ignore: "No paste apertures are generated for this package."
};

// Nur die zur gewählten Regel gehörenden Parameter anzeigen
function updateRuleFieldVisibility() {
    const sel = document.getElementById("selectPkgRule")?.value || "simple_rect";
    const rule = sel === "inherit" ? (pkgState.effectiveRuleType || "package_group") : sel;
    document.querySelectorAll(".rule-property-grid .prop-item[data-rules]").forEach(el => {
        el.classList.toggle("rule-hidden", !el.dataset.rules.split(" ").includes(rule));
    });
    const desc = document.getElementById("ruleDescription");
    if (desc) desc.textContent = RULE_DESCRIPTIONS[sel] || RULE_DESCRIPTIONS[rule] || "";
}

// Zahl aus Eingabefeld; leere/ungültige Eingabe → Standardwert (0 ist ein gültiger Wert!)
function numInput(id, def) {
    const v = parseFloat(document.getElementById(id)?.value);
    return Number.isFinite(v) ? v : def;
}

function collectRuleFromInputs() {
    return {
        rule_type: document.getElementById("selectPkgRule").value,
        corner_radius: numInput("propCornerRadius", 0),
        undersize_w: numInput("propUndersize", 0),
        undersize_h: numInput("propUndersize", 0),
        house_cut_pct: numInput("propHouseCut", 35),
        pitch_reduction_pct: numInput("propPitchRed", 20),
        length_usage_pct: numInput("propLengthUse", 95),
        gap_ratio: numInput("propGapRatio", 1.0),
        length_comp_pct: numInput("propLengthComp", 70),
        outward_shift: numInput("propOutwardShift", 0.025),
        offset_move_away: numInput("propOffsetMoveAway", 0),
        move_axis: document.getElementById("propMoveAxis")?.value || "long",
        offset_x: numInput("propOffsetX", 0),
        offset_y: numInput("propOffsetY", 0),
        bridge_count_x: Math.max(0, Math.round(numInput("propBridgeX", 0))),
        bridge_count_y: Math.max(0, Math.round(numInput("propBridgeY", 0))),
        bridge_width: numInput("propBridgeWidth", 0.3),
        rotate_bridges_45: document.getElementById("propRotate45").checked,
        try_find_direction: document.getElementById("propTryDirection").checked,
        toe_heel_shift: numInput("propToeHeelShift", 0),
        chamfer_size: numInput("propChamferSize", 0),
        chamfer_mode: document.getElementById("propChamferMode")?.value || "all",
        pin_minus_mask: Boolean(document.getElementById("propPinMinusMask")?.checked),
        size_w: numInput("propSizeW", 0),
        size_h: numInput("propSizeH", 0),
        donut_inner_dia: numInput("propDonutInner", 0),
        bga_threshold_square: numInput("propBgaThreshold", 0.41),
        chip_threshold_houses: numInput("propChipThreshold", 2.49),
        invert_house_below: numInput("propInvertHouse", 0.75),
        mini_chip_reduction: numInput("propMiniChip", 0.075),
        min_array_field: numInput("propMinArrayField", 0.75),
        distance_array_fields: numInput("propArrayDistance", 0.25),
        utilization_array_pct: numInput("propUtilArray", 40),
        utilization_house_pct: numInput("propUtilHouse", 0),
        pip_pcb_thickness: numInput("propPipPcb", 1.6),
        pip_metal_pct: numInput("propPipMetal", 50),
        pip_pin_clearance: numInput("propPipClearance", 0.25),
        pip_shape: document.getElementById("propPipShape")?.value || "circle",
        pip_max_size: numInput("propPipMax", 0),
        array_size_w: numInput("propArraySizeW", 0),
        array_size_h: numInput("propArraySizeH", 0),
        array_count_x: Math.max(1, Math.round(numInput("propArrayCountX", 2))),
        array_count_y: Math.max(1, Math.round(numInput("propArrayCountY", 2))),
        array_round: Boolean(document.getElementById("propArrayRound")?.checked),
        ignore_mask: Boolean(document.getElementById("propIgnoreMask")?.checked),
        donut_bridges: Math.max(1, Math.round(numInput("propDonutBridges", 4))),
        donut_rotation: numInput("propDonutRotation", 0),
        donut_inner_pct: numInput("propDonutInnerPct", 50),
        donut_outer_pct: numInput("propDonutOuterPct", 100),
        max_element_size: numInput("propMaxElement", 5),
        steg_big_elements: numInput("propStegBig", 0.75)
    };
}

let ruleUpdateTimeout = null;
function onRuleParamChanged(e) {
    noteRuleEdit();
    const select = document.getElementById("selectPkgRule");
    // Parameter an einer geerbten Regel geändert → eigene Regel mit dem geerbten Typ
    if (e && e.target && e.target.id !== "selectPkgRule" && select.value === "inherit") {
        select.value = pkgState.effectiveRuleType || "package_group";
        updateRuleFieldVisibility();
    }
    const ruleData = collectRuleFromInputs();
    const target = { ...(pkgState.ruleTarget || { kind: "package", id: pkgState.activePkgName }) };
    if (target.kind === "pin") {
        clearTimeout(ruleUpdateTimeout);
        ruleUpdateTimeout = setTimeout(() => savePinRule(target, ruleData), 60);
        return;
    }
    if (target.kind === "pins") {
        clearTimeout(ruleUpdateTimeout);
        ruleUpdateTimeout = setTimeout(() => savePinsRule(target, ruleData), 120);
        return;
    }

    clearTimeout(ruleUpdateTimeout);
    ruleUpdateTimeout = setTimeout(async () => {
        if (!target.id) return;
        const cur = pkgState.ruleTarget || { kind: "package", id: pkgState.activePkgName };
        if (cur.kind !== target.kind || cur.id !== target.id) return;
        const url = {
            package: `/api/package/${encodeURIComponent(target.id)}/rule`,
            group: `/api/group/${encodeURIComponent(target.id)}/rule`,
            zone: `/api/zone/${encodeURIComponent(target.id)}/rule`,
            component: `/api/component/${encodeURIComponent(target.id)}/rule`
        }[target.kind];
        const res = await fetch(url, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(ruleData) });
        const data = await res.json();
        const now = pkgState.ruleTarget || { kind: "package", id: pkgState.activePkgName };
        if (now.kind !== target.kind || now.id !== target.id) return; // veraltet
        if (target.kind === "package" && data.preview) {
            pkgState.activePackageData = data.preview;
            pkgState.effectiveRuleType = data.preview.effective_rule?.rule_type;
            if (data.preview.rule?.rule_type === "inherit") {
                // Auf "Inherit" umgestellt: Werte der Gruppenregel anzeigen
                populateRuleInputs({ ...data.preview.effective_rule, rule_type: "inherit" });
            }
            updateRuleSourceBanner();
            loadPackagesList();
            renderPkgPreview();
        } else if (target.kind === "component" && data.component) {
            pkgState.activePackageData = data.component.preview;
            pkgState.activeComponent = data.component;
            updateRuleSourceBanner();
            renderPkgPreview();
            renderComponentList();
        } else if (target.kind === "zone") {
            await refreshApertureData();
            renderZoneRuleList();
        } else if (target.kind === "group") {
            pkgState.groups = data.groups?.groups || pkgState.groups;
            drawPkgMessage(groupMessage(target.id));
        }
    }, 120);
}

// Live-Update für Einzelpad-Bearbeitung (0ms direkte Vorschau + 40ms Backend-Sync)
let singlePinDebounce = null;
function onSinglePinParamChanged() {
    if (pkgState.pinBarMode === "all" || pkgState.pinBarMode === "multi") {
        // Alle Pins / Mausauswahl: erst mit "Anpassen" übernehmen
        document.getElementById("btnSavePadOverride")?.classList.add("btn-pulse");
        return;
    }
    if (!pkgState.activePackageData || pkgState.selectedPin === null) return;
    const pinNum = pkgState.selectedPin;
    const ruleType = document.getElementById("selectPinRule")?.value || "inherit";
    updatePinBarMode();
    const built = buildPinPayload();
    const manual = ruleType === "manual";
    const w = manual ? built.size_w : built.w;
    const h = manual ? built.size_h : built.h;
    const red = manual ? 0 : built.undersize_w;

    if (isNaN(w) || isNaN(h) || w <= 0 || h <= 0) return;

    const pkgName = pkgState.activePkgName;

    if (!pkgState.activePackageData.pad_overrides) {
        pkgState.activePackageData.pad_overrides = {};
    }
    pkgState.activePackageData.pad_overrides[pinNum] = built;

    if (!manual && pkgState.activePackageData.paste) {
        for (const ap of pkgState.activePackageData.paste) {
            if (ap.pin === pinNum) {
                ap.w = Math.max(0.02, w - red * 2);
                ap.h = Math.max(0.02, h - red * 2);
                if (ap.pts && ap.pts.length >= 3) {
                    const minPx = Math.min(...ap.pts.map(pt => pt[0]));
                    const maxPx = Math.max(...ap.pts.map(pt => pt[0]));
                    const minPy = Math.min(...ap.pts.map(pt => pt[1]));
                    const maxPy = Math.max(...ap.pts.map(pt => pt[1]));
                    const oldW = (maxPx - minPx) || 1;
                    const oldH = (maxPy - minPy) || 1;
                    const scaleX = ap.w / oldW;
                    const scaleY = ap.h / oldH;
                    const cx = (minPx + maxPx) / 2;
                    const cy = (minPy + maxPy) / 2;
                    ap.pts = ap.pts.map(pt => [
                        cx + (pt[0] - cx) * scaleX,
                        cy + (pt[1] - cy) * scaleY
                    ]);
                }
            }
        }
    }

    // 0ms instant redraw!
    renderPkgPreview();

    // 2. Fast debounced sync to backend (40ms)
    clearTimeout(singlePinDebounce);
    singlePinDebounce = setTimeout(async () => {
        try {
            const payload = built;
            const res = await fetch(`/api/package/${encodeURIComponent(pkgName)}/pin/${pinNum}/override`, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(payload)
            });
            const data = await res.json();
            if (pkgName !== pkgState.activePkgName || pinNum !== pkgState.selectedPin) return; // veraltet
            if (data.preview) {
                pkgState.activePackageData = data.preview;
                populatePinDropdown(data.preview.pads, data.preview.pad_overrides);
                const dropdown = document.getElementById("selectPinDropdown");
                if (dropdown) dropdown.value = pinNum;
                // Regelpanel rechts zeigt denselben Stand wie das Popup
                if (pkgState.ruleTarget?.kind === "pin" && pkgState.ruleTarget.pin === pinNum) setPinRuleTarget(pinNum);
                renderPkgPreview();
                renderPkgTree();
            }
        } catch (e) {
            console.error("Fehler beim Live-Sync des Pin-Overrides:", e);
        }
    }, 40);
}

// -------------------------------------------------------------
// Package Canvas Renderer (PCB Investigator Style)
// -------------------------------------------------------------
// Pins ohne Kundenpaste (z. B. Zugentlastung): ohne Öffnung auf dem Board wirkt keine Regel
function renderPkgMissingPaste() {
    const box = document.getElementById("pkgMissingPaste");
    if (!box) return;
    const pins = pkgState.activePackageData?.missing_paste_pins || [];
    box.classList.toggle("hidden", pins.length === 0);
    document.getElementById("pkgMissingPasteText").textContent =
        `Pin ${pins.join(", ")} without customer paste – no aperture on the board there, rules have no effect.`;
    // Markierte Kupferpads (Klick / Strg+Klick / Rahmen) nur diese übernehmen, sonst alle
    const sel = selectedMissingPins();
    document.getElementById("btnPkgAddMissing").textContent = sel.length ? `Add pin ${sel.join(", ")} as paste` : "Add all as paste";
}

function selectedMissingPins() {
    const missing = pkgState.activePackageData?.missing_paste_pins || [];
    const sel = pkgState.pinBarMode === "multi" ? pkgState.selectedPins || [] : (pkgState.selectedPin != null ? [pkgState.selectedPin] : []);
    return sel.filter(p => missing.includes(p));
}

document.getElementById("btnPkgAddMissing")?.addEventListener("click", async () => {
    const name = pkgState.activePackageData?.name;
    if (!name) return;
    const sel = selectedMissingPins();
    const d = await (await fetch("/api/paste/add_missing", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(sel.length ? { package: name, pins: sel } : { package: name }) })).json();
    await refreshApertureData();
    await selectPackage(name);
    refreshHistory();
    showToast(`${d.added} apertures for ${name} generated from copper`);
});

function renderPkgPreview() {
    renderPkgMissingPaste();
    if (!pkgCtx || !pkgCanvas || !pkgState.activePackageData) return;
    const rect = pkgCanvas.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;

    // Reset Canvas & Dark Background (Schwarz wie in Screenshot 1 & 3)
    pkgCtx.save();
    pkgCtx.setTransform(1, 0, 0, 1, 0, 0);
    pkgCtx.fillStyle = "#000000";
    pkgCtx.fillRect(0, 0, pkgCanvas.width, pkgCanvas.height);
    pkgCtx.restore();

    pkgCtx.save();
    const centerX = w / 2 + pkgState.panX;
    const centerY = h / 2 + pkgState.panY;
    pkgCtx.translate(centerX, centerY);
    if (pkgState.rotation) {
        pkgCtx.rotate((pkgState.rotation * Math.PI) / 180);
    }
    pkgCtx.scale(pkgState.zoom, -pkgState.zoom); // Y nach oben wie in CAD
    { const t = pkgInstanceTf(); pkgCtx.transform(t.m * t.c, -t.s, t.m * t.s, t.c, 0, 0); }

    const data = pkgState.activePackageData;

    // 1. Koordinatenachsen / Nullpunkt
    pkgCtx.strokeStyle = "rgba(255, 255, 255, 0.12)";
    pkgCtx.lineWidth = 1 / pkgState.zoom;
    pkgCtx.beginPath();
    pkgCtx.moveTo(-10, 0); pkgCtx.lineTo(10, 0);
    pkgCtx.moveTo(0, -10); pkgCtx.lineTo(0, 10);
    pkgCtx.stroke();

    // 1b. Platinenumgebung (Nachbarn) wie PCB-I
    drawPkgBoardContext(pkgCtx, pkgState.zoom);

    // 2. Component Silkscreen / Body Outline (Gelb / Weiß)
    if (pkgState.showComponent && data.outline) {
        const ow = data.outline.w || 2.0;
        const oh = data.outline.h || 1.25;
        pkgCtx.strokeStyle = "rgba(250, 204, 21, 0.75)"; // Gelb
        pkgCtx.lineWidth = 1.5 / pkgState.zoom;
        pkgCtx.strokeRect(-ow/2, -oh/2, ow, oh);
    }

    // 2b. Lötstopp-Freistellung (Show Mask)
    if (pkgState.showMask && data.pads) {
        pkgCtx.fillStyle = "rgba(0, 168, 107, 0.28)";
        pkgCtx.strokeStyle = "rgba(16, 185, 129, 0.8)";
        pkgCtx.lineWidth = 1 / pkgState.zoom;
        pkgCtx.setLineDash([4 / pkgState.zoom, 3 / pkgState.zoom]);
        for (const p of data.pads) {
            if (!p.mask_w || !p.mask_h) continue;
            pkgCtx.beginPath();
            if (p.mask_round || p.shape === "circle") {
                pkgCtx.ellipse(p.cx, p.cy, p.mask_w / 2, p.mask_h / 2, 0, 0, Math.PI * 2);
            } else {
                pkgCtx.rect(p.cx - p.mask_w / 2, p.cy - p.mask_h / 2, p.mask_w, p.mask_h);
            }
            pkgCtx.fill();
            pkgCtx.stroke();
        }
        pkgCtx.setLineDash([]);
    }

    // 3. Copper Pads (Grün #00A86B / #10B981) - Exakt wie PCB Investigator Screenshot 1
    if (pkgState.showCopper && data.pads) {
        for (const p of data.pads) {
            const isSelected = isPinSelected(p.pin);
            pkgCtx.fillStyle = isSelected ? "rgba(250, 204, 21, 0.4)" : "#00a86b";
            pkgCtx.strokeStyle = isSelected ? "#facc15" : "#047857";
            pkgCtx.lineWidth = 1.2 / pkgState.zoom;
            if (p.shape === "circle") {
                pkgCtx.beginPath();
                pkgCtx.ellipse(p.cx, p.cy, p.w / 2, p.h / 2, 0, 0, Math.PI * 2);
                pkgCtx.fill();
                pkgCtx.stroke();
            } else {
                pkgCtx.fillRect(p.cx - p.w/2, p.cy - p.h/2, p.w, p.h);
                pkgCtx.strokeRect(p.cx - p.w/2, p.cy - p.h/2, p.w, p.h);
            }
        }
    }

    // 4. Stencil Paste Apertures (Translucent Slate Blue Fill + Lavender/Pink Stroke #C084FC)
    // Exakt wie PCB Investigator Screenshot 1 & 3!
    if (pkgState.showPaste && data.paste) {
        pkgCtx.fillStyle = "rgba(109, 132, 168, 0.75)"; // Blaue Pastenfüllung
        pkgCtx.strokeStyle = "#c084fc"; // Pinke/Lavendelfarbene Pastenkontur
        pkgCtx.lineWidth = 2.0 / pkgState.zoom;

        for (const ap of data.paste) {
            pkgCtx.strokeStyle = ap.type === "freehand" ? "#f59e0b" : "#c084fc"; // Skizzen orange umrandet
            if (ap.pts && ap.pts.length >= 3) {
                // Polygon (z. B. HouseBuilder, Rounded Rect, Notched)
                pkgCtx.beginPath();
                pkgCtx.moveTo(ap.pts[0][0], ap.pts[0][1]);
                for (let i = 1; i < ap.pts.length; i++) {
                    pkgCtx.lineTo(ap.pts[i][0], ap.pts[i][1]);
                }
                pkgCtx.closePath();
                pkgCtx.fill();
                pkgCtx.stroke();
            } else {
                // Einfaches Rechteck
                pkgCtx.fillRect(ap.cx - ap.w/2, ap.cy - ap.h/2, ap.w, ap.h);
                pkgCtx.strokeRect(ap.cx - ap.w/2, ap.cy - ap.h/2, ap.w, ap.h);
            }
        }
    }

    // 4b. Bohrungen (THT)
    if (pkgState.showDrills !== false && data.pads) {
        // Via/Bohrung als dünner Ring mit Kreuz (Lot kann ablaufen); keine Füllung, damit Pad und Paste sichtbar bleiben
        pkgCtx.strokeStyle = "rgba(251, 191, 36, 0.95)";
        pkgCtx.fillStyle = "rgba(0, 0, 0, 0)";
        pkgCtx.lineWidth = 1.2 / pkgState.zoom;
        for (const p of data.pads) {
            if (!p.drill) continue;
            const r = p.drill / 2;
            // An der echten Lage der Bohrung (Vias sitzen oft am Padrand), nicht in der Padmitte
            const [ox, oy] = p.drill_off || [0, 0];
            const x = p.cx + ox, y = p.cy + oy;
            pkgCtx.beginPath();
            pkgCtx.arc(x, y, r, 0, Math.PI * 2);
            pkgCtx.fill();
            pkgCtx.stroke();
            pkgCtx.beginPath();
            pkgCtx.moveTo(x - r * 0.6, y); pkgCtx.lineTo(x + r * 0.6, y);
            pkgCtx.moveTo(x, y - r * 0.6); pkgCtx.lineTo(x, y + r * 0.6);
            pkgCtx.stroke();
        }
    }

    // 5. Dimension Ruler & Pitch (Messlinie mit Pfeilen)
    if (pkgState.showRuler && data.pads && data.pads.length >= 2 && data.pitch > 0) {
        const p1 = data.pads[0];
        const p2 = data.pads[1];
        const yOff = Math.max(p1.cy + p1.h/2, p2.cy + p2.h/2) + 0.35;

        pkgCtx.strokeStyle = "#38bdf8";
        pkgCtx.lineWidth = 1.5 / pkgState.zoom;

        // Linie zwischen Pad-Zentren
        pkgCtx.beginPath();
        pkgCtx.moveTo(p1.cx, yOff);
        pkgCtx.lineTo(p2.cx, yOff);
        // Ticks
        pkgCtx.moveTo(p1.cx, yOff - 0.15); pkgCtx.lineTo(p1.cx, yOff + 0.15);
        pkgCtx.moveTo(p2.cx, yOff - 0.15); pkgCtx.lineTo(p2.cx, yOff + 0.15);
        pkgCtx.stroke();
    }

    drawFreehandOverlay(pkgCtx, data, pkgState.zoom);

    // 6. Pin Labels (falls aktiviert)
    pkgCtx.restore();
    drawSketchLabel(pkgCtx);

    if (pkgState.showPinLabels && data.pads) {
        pkgCtx.save();
        resetToCssPixels(pkgCtx);
        pkgCtx.fillStyle = "#ffffff";
        pkgCtx.font = "11px monospace";
        pkgCtx.textAlign = "center";
        pkgCtx.textBaseline = "middle";
        for (const p of data.pads) {
            const sp = pkgWorldToScreen(p.cx, p.cy);
            pkgCtx.fillText(p.pin, sp.x, sp.y);
        }
        pkgCtx.restore();
    }

    // Measurements (Package View)
    drawMeasurementLayer(pkgCtx, "package", pkgWorldToScreen, pkgState.zoom);

    // Info-Layer: AR / Fläche / Volumen je Öffnung
    renderPasteInfo(data);

    // Dynamic CAD Scale Bar (Maßstab)
    drawCanvasScaleBar(pkgCtx, w, h, pkgState.zoom, 14, 38);
    // Auswahlrahmen (Maus ziehen)
    if (pkgState.box) {
        const b = pkgState.box;
        pkgCtx.save();
        resetToCssPixels(pkgCtx);
        pkgCtx.strokeStyle = "#0078d4";
        pkgCtx.fillStyle = "rgba(0, 120, 212, 0.12)";
        pkgCtx.lineWidth = 1;
        pkgCtx.setLineDash([4, 3]);
        const x = Math.min(b.x0, b.x1), y = Math.min(b.y0, b.y1), w = Math.abs(b.x1 - b.x0), h = Math.abs(b.y1 - b.y0);
        pkgCtx.fillRect(x, y, w, h);
        pkgCtx.strokeRect(x, y, w, h);
        pkgCtx.restore();
    }
}

// Pin-Editor im Modus "Alle Pins": Regel/Größe/Reduktion für das ganze Package
function openAllPinsBar() {
    const data = pkgState.activePackageData;
    if (!data) return;
    if (pkgState.ruleTarget?.kind === "pin" || pkgState.ruleTarget?.kind === "pins") setPackageRuleTarget();
    pkgState.selectedPin = null;
    pkgState.selectedPins = [];
    pkgState.pinBarMode = "all";
    const bar = document.getElementById("singlePadBar");
    bar.classList.remove("hidden");
    const pads = data.pads || [];
    document.getElementById("lblPinBarPrefix").textContent = "";
    document.getElementById("lblSelectedPin").textContent = `All Pins (${pads.length})`;
    document.getElementById("btnApplyPinToAll")?.classList.add("hidden");
    const dropdown = document.getElementById("selectPinDropdown");
    if (dropdown) dropdown.value = "all";

    // Gemeinsame Werte vorbelegen, unterschiedliche Werte leer lassen
    const ovs = Object.values(data.pad_overrides || {});
    const common = (vals) => vals.length > 0 && vals.every(v => v === vals[0]) ? vals[0] : null;
    const ruleSelect = document.getElementById("selectPinRule");
    if (ruleSelect) ruleSelect.value = (ovs.length === pads.length && common(ovs.map(o => o.rule_type || "inherit"))) || "inherit";
    const inpW = document.getElementById("inputPadCustomW");
    const inpH = document.getElementById("inputPadCustomH");
    const cw = common(pads.map(p => +p.w.toFixed(3)));
    const ch = common(pads.map(p => +p.h.toFixed(3)));
    const [sw, sh] = sizeForScreen(cw, ch);
    inpW.value = sw ?? "";
    inpH.value = sh ?? "";
    inpW.placeholder = sw === null ? "mixed" : "B (mm)";
    inpH.placeholder = sh === null ? "mixed" : "H (mm)";
    document.getElementById("inputPadOffX").value = 0;
    document.getElementById("inputPadOffY").value = 0;
    updatePinBarMode();
    const undersizeInput = document.getElementById("inputPadCustomUndersize");
    if (undersizeInput) undersizeInput.value = common(ovs.map(o => o.undersize_w)) ?? data.rule?.undersize_w ?? 0.025;
    renderPkgPreview();
}

// Sammel-Übersteuerung für alle Pins (matchPin: Breite/Höhe nur auf gleich große Pads übertragen)
async function applyPinsBulk(override, w, h, matchPin = null, pins = null) {
    const pkgName = pkgState.activePkgName;
    if (!pkgName) return null;
    document.getElementById("btnSavePadOverride")?.classList.remove("btn-pulse");
    const res = await fetch(`/api/package/${encodeURIComponent(pkgName)}/pins/override`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ override, w: isNaN(w) ? null : w, h: isNaN(h) ? null : h, match_pin: matchPin, pins })
    });
    const data = await res.json();
    if (pkgName !== pkgState.activePkgName) return data; // inzwischen anderes Package gewählt
    if (data.preview) {
        pkgState.activePackageData = data.preview;
        populatePinDropdown(data.preview.pads, data.preview.pad_overrides);
        const dropdown = document.getElementById("selectPinDropdown");
        if (dropdown) dropdown.value = pkgState.selectedPin ?? "all";
        renderPkgPreview();
    }
    return data;
}

// Pins mit gleicher Kupfergröße und Form wie der Referenzpin (eine "Pad-Gruppe")
function samePadPins(refPin) {
    const pads = pkgState.activePackageData?.pads || [];
    const ref = pads.find(p => p.pin === refPin);
    if (!ref) return [refPin];
    return pads.filter(p => Math.abs(p.w - ref.w) < 0.005 && Math.abs(p.h - ref.h) < 0.005 && (p.shape || "rect") === (ref.shape || "rect")).map(p => p.pin);
}

// Mehrere Pins ausgewählt (Strg+Klick / Rahmen): Popup und Regelpanel gelten nur für diese Pins
function selectPkgPins(pins) {
    const d = pkgState.activePackageData;
    if (!d) return;
    pins = [...new Set(pins)].filter(p => d.pads.some(x => x.pin === p));
    if (pins.length === 0) { openAllPinsBar(); renderPkgTree(); return; }
    if (pins.length === 1) { const pad = d.pads.find(p => p.pin === pins[0]); selectPkgPin(pad.pin, pad); renderPkgTree(); return; }
    pkgState.selectedPin = null;
    pkgState.selectedPins = pins;
    pkgState.pinBarMode = "multi";
    state.highlightPin = null;
    document.getElementById("singlePadBar").classList.remove("hidden");
    document.getElementById("lblPinBarPrefix").textContent = "";
    document.getElementById("lblSelectedPin").textContent = `${pins.length} pins selected`;
    document.getElementById("btnApplyPinToAll")?.classList.add("hidden");
    const dd = document.getElementById("selectPinDropdown");
    if (dd) dd.value = "all";
    const pads = d.pads.filter(p => pins.includes(p.pin));
    const ovs = pins.map(pinOverrideOf);
    const common = (vals) => vals.length > 0 && vals.every(v => v === vals[0]) ? vals[0] : null;
    const rt = common(ovs.map(o => (o?.manual ? "manual" : o?.rule_type) || "inherit"));
    document.getElementById("selectPinRule").value = rt || "inherit";
    const [sw, sh] = sizeForScreen(common(pads.map(p => +p.w.toFixed(3))), common(pads.map(p => +p.h.toFixed(3))));
    const inpW = document.getElementById("inputPadCustomW"), inpH = document.getElementById("inputPadCustomH");
    inpW.value = sw ?? ""; inpH.value = sh ?? "";
    inpW.placeholder = sw === null ? "mixed" : "B (mm)"; inpH.placeholder = sh === null ? "mixed" : "H (mm)";
    updatePinBarMode();
    // Regelpanel rechts auf die Auswahl richten
    const base = d.rule?.rule_type === "inherit" ? d.effective_rule : d.rule;
    pkgState.ruleTarget = { kind: "pins", id: d.name, pins };
    const first = ovs.find(o => o && o.rule_type && o.rule_type !== "inherit");
    populateRuleInputs(rt && rt !== "inherit" && first ? { ...base, ...first } : { ...base, rule_type: "inherit" });
    updateRuleSourceBanner();
    renderPkgPreview();
    renderPkgTree();
}

function isPinSelected(pin) {
    return pkgState.selectedPin === pin || (pkgState.pinBarMode === "multi" && (pkgState.selectedPins || []).includes(pin));
}

// Regel für die Mausauswahl speichern ("Wie Package" = Anpassungen der Auswahl entfernen)
async function savePinsRule(target, ruleData) {
    noteRuleEdit();
    const cur = pkgState.ruleTarget;
    if (!cur || cur.kind !== "pins" || cur.id !== target.id) return;
    let data;
    if (ruleData.rule_type === "inherit") {
        data = await (await fetch(`/api/package/${encodeURIComponent(target.id)}/pins/reset`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ pins: target.pins }) })).json();
    } else {
        data = await applyPinsBulk(ruleData, NaN, NaN, null, target.pins);
    }
    if (data?.preview) {
        pkgState.activePackageData = data.preview;
        populatePinDropdown(data.preview.pads, data.preview.pad_overrides);
        updateRuleSourceBanner();
        renderPkgPreview();
        renderPkgTree();
    }
    await refreshApertureData();
    refreshHistory();
}

function readPinBarValues() {
    return {
        ruleType: document.getElementById("selectPinRule")?.value || "inherit",
        w: parseFloat(document.getElementById("inputPadCustomW")?.value),
        h: parseFloat(document.getElementById("inputPadCustomH")?.value),
        red: parseFloat(document.getElementById("inputPadCustomUndersize")?.value),
        dx: parseFloat(document.getElementById("inputPadOffX")?.value) || 0,
        dy: parseFloat(document.getElementById("inputPadOffY")?.value) || 0
    };
}

// Die Pin-Felder zeigen Maße so, wie sie in der (ggf. gedrehten) Vorschau erscheinen:
// B = waagerecht, H = senkrecht, dX nach rechts, dY nach oben.
function pkgViewSwapped() {
    const r = (((pkgState.rotation || 0) % 180) + 180) % 180;
    return Math.abs(r - 90) < 1;
}
function sizeForScreen(w, h) {
    return pkgViewSwapped() ? [h, w] : [w, h];
}
function pkgVecToScreen(x, y) {
    const a = ((pkgState.rotation || 0) * Math.PI) / 180, c = Math.cos(a), s = Math.sin(a);
    return { x: x * c + y * s, y: -(x * s - y * c) };
}
function screenVecToPkg(x, yUp) {
    const a = ((pkgState.rotation || 0) * Math.PI) / 180, c = Math.cos(a), s = Math.sin(a), vy = -yUp;
    return { x: x * c + vy * s, y: x * s - vy * c };
}
const r4js = (v) => Math.round(v * 10000) / 10000;

function updatePinBarMode() {
    const manual = document.getElementById("selectPinRule")?.value === "manual";
    const slicing = document.getElementById("selectPinRule")?.value === "array_slicing";
    document.querySelectorAll(".pin-manual-only").forEach(el => el.classList.toggle("hidden", !manual));
    document.querySelectorAll(".pin-slice-only").forEach(el => el.classList.toggle("hidden", !slicing));
    document.querySelectorAll(".pin-rule-only").forEach(el => el.classList.toggle("hidden", manual));
    const inpW = document.getElementById("inputPadCustomW");
    const inpH = document.getElementById("inputPadCustomH");
    if (inpW && !inpW.placeholder.startsWith("mixed")) inpW.placeholder = manual ? "Aperture W" : "Pad W";
    if (inpH && !inpH.placeholder.startsWith("mixed")) inpH.placeholder = manual ? "Aperture H" : "Pad H";
    if (inpW) inpW.title = manual ? "Width of paste aperture (horizontal as shown)" : "Pad width from which the rule computes the aperture (horizontal as shown)";
    if (inpH) inpH.title = manual ? "Height of paste aperture (vertical as shown)" : "Pad height from which the rule computes the aperture (vertical as shown)";
}

// Übersteuerung aus den Feldern (Package-Rahmen). Manuell = feste Öffnung mit Versatz.
function buildPinPayload() {
    const v = readPinBarValues();
    const [pw, ph] = sizeForScreen(v.w, v.h);
    if (v.ruleType === "manual") {
        const off = screenVecToPkg(v.dx, v.dy);
        const base = pkgState.activePackageData?.effective_rule || pkgState.activePackageData?.rule || {};
        return {
            rule_type: "simple_rect", manual: true,
            size_w: r4js(pw), size_h: r4js(ph), offset_x: r4js(off.x), offset_y: r4js(off.y),
            undersize_w: 0, undersize_h: 0, offset_move_away: 0, toe_heel_shift: 0,
            bridge_count_x: 0, bridge_count_y: 0, chamfer_size: 0, pin_minus_mask: false,
            corner_radius: base.corner_radius ?? 0.075
        };
    }
    const red = isNaN(v.red) ? 0.025 : v.red;
    const payload = { rule_type: v.ruleType, w: pw, h: ph, undersize_w: red, undersize_h: red };
    if (v.ruleType === "array_slicing") {
        // Raster wie angezeigt → Package-Rahmen (bei gedrehter Ansicht Spalten/Zeilen tauschen)
        let [nx, ny] = (document.getElementById("pinSliceGrid")?.value || "2x2").split("x").map(Number);
        if (pkgViewSwapped()) [nx, ny] = [ny, nx];
        payload.bridge_count_x = Math.max(0, nx - 1);
        payload.bridge_count_y = Math.max(0, ny - 1);
        payload.bridge_width = parseFloat(document.getElementById("pinSliceWeb")?.value) || 0.25;
        payload.rotate_bridges_45 = false;
    }
    return payload;
}

function fillPinBarFromOverride(ov, padObj) {
    const ruleSelect = document.getElementById("selectPinRule");
    if (ruleSelect) ruleSelect.value = ov?.manual ? "manual" : (ov?.rule_type || "inherit");
    let w, h;
    if (ov?.manual) {
        [w, h] = sizeForScreen(ov.size_w, ov.size_h);
        const off = pkgVecToScreen(ov.offset_x || 0, ov.offset_y || 0);
        document.getElementById("inputPadOffX").value = r4js(off.x);
        document.getElementById("inputPadOffY").value = r4js(off.y);
    } else {
        [w, h] = sizeForScreen(ov?.w ?? padObj.w, ov?.h ?? padObj.h);
        document.getElementById("inputPadOffX").value = 0;
        document.getElementById("inputPadOffY").value = 0;
    }
    if (ov?.rule_type === "array_slicing") {
        let [nx, ny] = [(ov.bridge_count_x ?? 1) + 1, (ov.bridge_count_y ?? 1) + 1];
        if (pkgViewSwapped()) [nx, ny] = [ny, nx];
        const grid = document.getElementById("pinSliceGrid");
        if (grid && [...grid.options].some(o => o.value === `${nx}x${ny}`)) grid.value = `${nx}x${ny}`;
        document.getElementById("pinSliceWeb").value = ov.bridge_width ?? 0.25;
    }
    document.getElementById("inputPadCustomW").value = r4js(w);
    document.getElementById("inputPadCustomH").value = r4js(h);
    updatePinBarMode();
}
window.fillPinBarFromOverride = fillPinBarFromOverride;

function selectPkgPin(pinNum, padObj) {
    pkgState.selectedPin = pinNum;
    pkgState.selectedPins = [pinNum];
    pkgState.pinBarMode = "single";
    state.highlightPin = pinNum;
    window.scBroadcast?.("pkg-focus", { pkg: pkgState.activePkgName, pin: pinNum });
    const bar = document.getElementById("singlePadBar");
    bar.classList.remove("hidden");
    document.getElementById("lblPinBarPrefix").textContent = "Pin ";
    document.getElementById("lblSelectedPin").textContent = pinNum;
    document.getElementById("btnApplyPinToAll")?.classList.remove("hidden");
    document.getElementById("inputPadCustomW").placeholder = "B (mm)";
    document.getElementById("inputPadCustomH").placeholder = "H (mm)";
    document.querySelectorAll(".pkg-pin-item").forEach(el => el.classList.toggle("active", el.firstChild?.textContent?.startsWith(`${pinNum}(`)));

    const dropdown = document.getElementById("selectPinDropdown");
    if (dropdown) dropdown.value = pinNum;

    const overrides = pkgState.activePackageData ? pkgState.activePackageData.pad_overrides : {};
    const ov = (overrides && (overrides[pinNum] || overrides[String(pinNum)])) ? (overrides[pinNum] || overrides[String(pinNum)]) : null;

    fillPinBarFromOverride(ov, padObj);
    const undersizeInput = document.getElementById("inputPadCustomUndersize");
    if (undersizeInput) undersizeInput.value = ov?.undersize_w ?? pkgState.activePackageData?.rule?.undersize_w ?? 0.025;
    setPinRuleTarget(pinNum);

    renderPkgPreview();
}

// Regelpanel auf einen einzelnen Pin richten: Auswahl und Parameter gelten nur für ihn
function pinOverrideOf(pin) {
    const ovs = pkgState.activePackageData?.pad_overrides || {};
    return ovs[pin] || ovs[String(pin)] || null;
}

function setPinRuleTarget(pin) {
    const d = pkgState.activePackageData;
    if (!d) return;
    pkgState.ruleTarget = { kind: "pin", id: d.name, pin };
    const base = d.rule?.rule_type === "inherit" ? d.effective_rule : d.rule;
    const ov = pinOverrideOf(pin);
    const own = ov && ov.rule_type && ov.rule_type !== "inherit";
    populateRuleInputs(own ? { ...base, ...ov } : { ...base, rule_type: "inherit" });
    updateRuleSourceBanner();
}

function setPackageRuleTarget() {
    const d = pkgState.activePackageData;
    if (!d) return;
    pkgState.ruleTarget = { kind: "package", id: d.name };
    populateRuleInputs(d.rule?.rule_type === "inherit" ? { ...d.effective_rule, rule_type: "inherit" } : d.rule);
    updateRuleSourceBanner();
}
window.setPackageRuleTarget = setPackageRuleTarget;

// -------------------------------------------------------------
// Package Event Listeners
// -------------------------------------------------------------
function setupPkgEventListeners() {
    // Search & Filter
    const searchInput = document.getElementById("pkgSearchInput");
    searchInput.addEventListener("input", (e) => {
        pkgState.searchQuery = e.target.value;
        renderPkgTree();
    });
    document.getElementById("btnClearPkgSearch").addEventListener("click", () => {
        searchInput.value = "";
        pkgState.searchQuery = "";
        renderPkgTree();
    });

    document.querySelectorAll(".cat-pill").forEach(pill => {
        pill.addEventListener("click", () => {
            document.querySelectorAll(".cat-pill").forEach(p => p.classList.remove("active"));
            pill.classList.add("active");
            pkgState.activeCategory = pill.dataset.cat;
            renderPkgTree();
        });
    });

    // Rule Inputs live sync (0ms / 35ms schnelles Live-Update)
    document.getElementById("selectPkgRule").addEventListener("change", () => { updateRuleFieldVisibility(); onRuleParamChanged(); });
    [
        "propCornerRadius", "propUndersize", "propHouseCut", "propPitchRed", "propLengthUse", "propGapRatio", "propLengthComp", "propOutwardShift", "propMoveAxis",
        "propOffsetMoveAway", "propOffsetX", "propOffsetY", "propBridgeX", "propBridgeY", "propBridgeWidth",
        "propToeHeelShift", "propChamferSize", "propSizeW", "propSizeH", "propDonutInner",
        "propBgaThreshold", "propChipThreshold", "propInvertHouse", "propMiniChip",
        "propMinArrayField", "propArrayDistance", "propUtilArray", "propUtilHouse",
        "propPipPcb", "propPipMetal", "propPipClearance", "propPipMax", "propPipShape",
        "propArraySizeW", "propArraySizeH", "propArrayCountX", "propArrayCountY", "propDonutBridges", "propDonutRotation",
        "propDonutInnerPct", "propDonutOuterPct", "propMaxElement", "propStegBig"
    ].forEach(id => {
        const el = document.getElementById(id);
        if (el) {
            el.addEventListener("input", onRuleParamChanged);
            el.addEventListener("change", onRuleParamChanged);
        }
    });
    document.getElementById("propRotate45")?.addEventListener("change", onRuleParamChanged);
    document.getElementById("propArrayRound")?.addEventListener("change", onRuleParamChanged);
    document.getElementById("propIgnoreMask")?.addEventListener("change", onRuleParamChanged);
    // Parameter alphabetisch wie im PCB-I-Eigenschaftenfenster, Schalter am Ende; Infotext darunter
    const grid = document.querySelector(".rule-property-grid");
    const desc = document.getElementById("ruleDescription");
    if (grid && desc) grid.after(desc);
    if (grid) {
        const items = [...grid.querySelectorAll(":scope > .prop-item")];
        const box = el => el.classList.contains("checkbox-prop") ? 1 : 0;
        const label = el => (el.querySelector("label")?.textContent || "").trim();
        items.sort((a, b) => box(a) - box(b) || label(a).localeCompare(label(b), "en", { sensitivity: "base" })).forEach(el => grid.appendChild(el));
    }
    document.getElementById("propPinMinusMask")?.addEventListener("change", onRuleParamChanged);
    document.getElementById("chkPrevBoard")?.addEventListener("change", loadPkgBoardContext);
    document.getElementById("selPrevBoardRadius")?.addEventListener("change", loadPkgBoardContext);
    document.getElementById("propChamferMode")?.addEventListener("change", onRuleParamChanged);
    document.getElementById("propTryDirection")?.addEventListener("change", onRuleParamChanged);

    // Auto-Klassifizierung
    async function triggerAutoClassify(btnElement) {
        const orig = btnElement ? btnElement.innerHTML : "";
        if (btnElement) {
            btnElement.innerHTML = `<span></span> Classifying...`;
            btnElement.disabled = true;
        }
        try {
            const res = await fetch("/api/auto_classify", { method: "POST" });
            const data = await res.json();
            if (data.status === "ok") {
                const c = data.classification;
                showToast(`Auto classification done! ${c.chips} Chips, ${c.soic} SOIC, ${c.qfn} QFN, ${c.bga} BGA, ${c.tht} THT`);
                await loadPackagesList();
                await refreshApertureData();
                if (pkgState.activePkgName) {
                    selectPackage(pkgState.activePkgName);
                }
            }
        } catch (e) {
            console.error("Auto-Klassifizierung Fehler:", e);
        } finally {
            if (btnElement) {
                btnElement.innerHTML = orig;
                btnElement.disabled = false;
            }
        }
    }

    document.getElementById("btnAutoClassify")?.addEventListener("click", (e) => triggerAutoClassify(e.currentTarget));
    document.getElementById("btnHeaderAutoClassify")?.addEventListener("click", (e) => triggerAutoClassify(e.currentTarget));

    // Single Pad Live Inputs (0ms Live-Update beim Hoch/Runter-Klicken der Spinner!)
    [
        "inputPadCustomW", "inputPadCustomH", "inputPadCustomUndersize", "inputPadOffX", "inputPadOffY", "pinSliceGrid", "pinSliceWeb"
    ].forEach(id => {
        const el = document.getElementById(id);
        if (el) {
            el.addEventListener("input", onSinglePinParamChanged);
            el.addEventListener("change", onSinglePinParamChanged);
        }
    });
    document.getElementById("selectPinRule")?.addEventListener("change", onSinglePinParamChanged);

    // Pfeiltasten verschieben die manuelle Öffnung des gewählten Pins (Umschalt = 0,05 mm)
    window.addEventListener("keydown", (e) => {
        if (!e.key.startsWith("Arrow")) return;
        const bar = document.getElementById("singlePadBar");
        if (!bar || bar.classList.contains("hidden") || pkgState.pinBarMode !== "single") return;
        if (document.getElementById("selectPinRule")?.value !== "manual") return;
        const tag = (document.activeElement?.tagName || "").toLowerCase();
        if (tag === "input" || tag === "select" || tag === "textarea") return;
        const step = e.shiftKey ? 0.05 : 0.01;
        const fx = document.getElementById("inputPadOffX"), fy = document.getElementById("inputPadOffY");
        const dx = { ArrowLeft: -step, ArrowRight: step }[e.key] || 0;
        const dy = { ArrowDown: -step, ArrowUp: step }[e.key] || 0;
        fx.value = r4js((parseFloat(fx.value) || 0) + dx);
        fy.value = r4js((parseFloat(fy.value) || 0) + dy);
        e.preventDefault();
        e.stopPropagation();
        onSinglePinParamChanged();
    }, true);

    // Measurement Tool in Stencil Generator
    document.getElementById("btnMeasurePkg")?.addEventListener("click", () => toggleMeasureTool());
    // Zoom um die Mitte der Vorschau (Ansicht ist mittenbezogen: Pan skaliert mit)
    const pkgZoom = (f) => { pkgState.zoom = Math.max(10, Math.min(400, pkgState.zoom * f)); pkgState.panX *= f; pkgState.panY *= f; renderPkgPreview(); };
    document.getElementById("btnPkgZoomIn")?.addEventListener("click", () => pkgZoom(1.3));
    document.getElementById("btnPkgZoomOut")?.addEventListener("click", () => pkgZoom(1 / 1.3));
    document.getElementById("btnPkgFit")?.addEventListener("click", () => { if (pkgState.activePackageData) { autoFitPackagePreview(pkgState.activePackageData); renderPkgPreview(); } });
    document.getElementById("btnCancelMeasurePkg")?.addEventListener("click", () => toggleMeasureTool(false));
    document.getElementById("btnClearMeasurementsPkg")?.addEventListener("click", clearMeasurements);
    document.getElementById("btnClearMeasurePkg")?.addEventListener("click", clearMeasurements);

    // Package 90° drehen
    document.getElementById("btnRotatePkg")?.addEventListener("click", () => {
        pkgState.rotation = ((pkgState.rotation || 0) + 90) % 360;
        if (pkgState.selectedPin !== null && pkgState.pinBarMode === "single") {
            const pad = pkgState.activePackageData?.pads?.find(p => p.pin === pkgState.selectedPin);
            const ovs = pkgState.activePackageData?.pad_overrides || {};
            if (pad) fillPinBarFromOverride(ovs[pad.pin] || ovs[String(pad.pin)] || null, pad);
        }
        showToast(`⟳ Package rotated: ${pkgState.rotation}°`);
        renderPkgPreview();
    });

    // Layer Toggles
    document.getElementById("chkPrevMask").addEventListener("change", e => { pkgState.showMask = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevCopper").addEventListener("change", e => { pkgState.showCopper = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevPaste").addEventListener("change", e => { pkgState.showPaste = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevCustomer")?.addEventListener("change", () => renderPkgPreview());
    document.getElementById("chkPrevComponent").addEventListener("change", e => { pkgState.showComponent = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevRuler").addEventListener("change", e => { pkgState.showRuler = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevPinLabels").addEventListener("change", e => { pkgState.showPinLabels = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevDrills")?.addEventListener("change", e => { pkgState.showDrills = e.target.checked; renderPkgPreview(); });
    // Schalter ändern die Lagen der Seite mit (Board View und Lagenliste sehen dasselbe)
    for (const [id, test] of Object.entries(PREVIEW_LAYERS)) {
        document.getElementById(id)?.addEventListener("change", (e) => {
            const ls = previewSideLayers(test);
            if (!ls.length || ls.every(l => (l.visible !== false) === e.target.checked)) return;
            for (const l of ls) setLayerVisible(l, e.target.checked);
            renderLayerTable();
            render();
        });
    }
    document.getElementById("chkPrevInfo")?.addEventListener("change", e => { pkgState.showInfo = e.target.checked; renderPkgPreview(); });
    setupRuleTargets();

    // Generate Stencil Pads: ganze Platine (TOP + BOT) nach dem Regelbaum, wie PCB-I.
    // Einzelnes Package / Gruppe: Rechtsklick im Baum ("Nur … generieren").
    document.getElementById("btnGenerateStencilPads").addEventListener("click", async () => {
        const btn = document.getElementById("btnGenerateStencilPads");
        const originalText = btn.innerHTML;
        btn.disabled = true;
        btn.innerHTML = `<span></span> Generating...`;
        try {
            await new Promise(r => setTimeout(r, 80)); // laufende Regel-Speicherung abwarten
            const res = await fetch("/api/generate_all", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
            if (!res.ok) throw new Error(`HTTP ${res.status}`);
            const data = await res.json();
            await afterGenerate();
            showToast(`${data.modified_count} pads generated on TOP and BOT (top_stencil / bot_stencil)`);
        } catch (e) {
            console.error("Fehler bei Generate Stencil Pads:", e);
            showToast(`Generate failed: ${e.message || e}`, 6000);
        } finally {
            btn.innerHTML = originalText;
            btn.disabled = false;
        }
    });

    // Customer Library Actions
    document.getElementById("btnSaveCurrentLib").addEventListener("click", async () => {
        const cust = pkgState.currentCustomer;
        if (await saveLibrary(cust)) {
            pkgState.libEdited = false;
            renderLibHeader();
            showToast(`Customer library "${cust}" saved`);
        }
    });

    // Laden-Menü und Nachfrage-Dialog
    const loadMenu = document.getElementById("libLoadList");
    document.getElementById("btnLibLoad")?.addEventListener("click", (e) => { e.stopPropagation(); loadMenu?.classList.toggle("hidden"); });
    loadMenu?.addEventListener("click", () => loadMenu.classList.add("hidden"));
    document.addEventListener("click", (e) => { if (!e.target.closest?.("#libLoadMenu")) loadMenu?.classList.add("hidden"); });
    document.getElementById("btnLibAskSkip")?.addEventListener("click", () => document.getElementById("libAskModal").classList.add("hidden"));
    document.getElementById("btnLibAskPc")?.addEventListener("click", () => { document.getElementById("libAskModal").classList.add("hidden"); loadLibraryFlow({ pick: true }); });

    // Werkzeug-Menü der Bibliothek
    const libMenu = document.getElementById("libToolsList");
    document.getElementById("btnLibTools")?.addEventListener("click", (e) => {
        e.stopPropagation();
        libMenu?.classList.toggle("hidden");
    });
    libMenu?.addEventListener("click", () => libMenu.classList.add("hidden"));
    document.addEventListener("click", (e) => {
        if (!e.target.closest?.("#libToolsMenu")) libMenu?.classList.add("hidden");
    });

    // PCB-Investigator-Kundenbibliothek (nur Desktop: Dateidialoge)
    if (!window.__TAURI__) document.querySelectorAll(".desktop-only").forEach(el => el.classList.add("hidden"));
    document.getElementById("btnPcbiImport")?.addEventListener("click", importPcbiLibrary);
    document.getElementById("btnPcbiExport")?.addEventListener("click", async () => {
        const res = await fetch("/api/pcbi/export", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
        const d = await res.json();
        if (d.status === "cancelled") return;
        if (!res.ok || d.status !== "ok") { showToast(`${d.message || d.error || "Export failed"}`); return; }
        let msg = `Saved: ${d.path}

${d.packages} package rules, ${d.pin_rules} pin rules, ${d.groups} groups, ${d.components} component rules.`;
        if (d.skipped?.length) msg += `

No PCB-I equivalent (not exported):
• ${d.skipped.slice(0, 12).join("\n• ")}${d.skipped.length > 12 ? `
… and ${d.skipped.length - 12} more` : ""}`;
        await askChoice("PCB-I library exported", msg, [{ label: "OK", value: "ok", cls: "btn-primary" }]);
    });
    document.getElementById("btnAddMissingPaste")?.addEventListener("click", () => offerMissingPaste(true));

    const newLibModal = document.getElementById("newLibModal");
    document.getElementById("btnOpenNewLibModal")?.addEventListener("click", () => {
        newLibModal.classList.remove("hidden");
    });
    document.getElementById("btnCancelNewLib").addEventListener("click", () => {
        newLibModal.classList.add("hidden");
    });
    document.getElementById("btnConfirmNewLib").addEventListener("click", async () => {
        const name = document.getElementById("inputNewCustName").value.trim();
        if (!name) return;
        if (!(await saveLibrary(name))) return;
        newLibModal.classList.add("hidden");
        await loadLibraries();
        pkgState.currentCustomer = name;
        pkgState.libEdited = false;
        renderLibHeader();
        showToast(`New library "${name}" created`);
    });

    // Pin Selector Dropdown
    const selectPinDropdown = document.getElementById("selectPinDropdown");
    if (selectPinDropdown) {
        selectPinDropdown.addEventListener("change", (e) => {
            const val = e.target.value;
            if (val.startsWith("sketch:")) {
                sk.sel = +val.slice(7);
                e.target.value = pkgState.selectedPin ?? "all";
                renderPkgPreview();
                return;
            }
            if (val === "all") {
                openAllPinsBar();
            } else {
                const pinNum = parseInt(val, 10);
                const pad = pkgState.activePackageData?.pads?.find(p => p.pin === pinNum);
                if (pad) {
                    selectPkgPin(pinNum, pad);
                }
            }
        });
    }

    // Package Rename Button
    const btnRenamePackage = document.getElementById("btnRenamePackage");
    if (btnRenamePackage) {
        btnRenamePackage.addEventListener("click", async () => {
            if (!pkgState.activePkgName) return;
            const oldName = pkgState.activePkgName;
            const newName = prompt(`New name for package "${oldName}":`, oldName);
            if (!newName || newName.trim() === "" || newName.trim() === oldName) return;

            try {
                const res = await fetch(`/api/package/${encodeURIComponent(oldName)}/rename`, {
                    method: "POST",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify({ new_name: newName.trim() })
                });
                const data = await res.json();
                if (data.status === "ok") {
                    await loadPackagesList();
                    selectPackage(newName.trim());
                    await refreshApertureData();
                    showToast(`Package renamed to "${newName.trim()}"`);
                } else {
                    alert(data.message || "Rename failed");
                }
            } catch (e) {
                console.error("Fehler beim Umbenennen:", e);
            }
        });
    }

    // Single Pad Override Actions (Speichern & Reset)
    const btnSavePadOverride = document.getElementById("btnSavePadOverride");
    if (btnSavePadOverride) {
        btnSavePadOverride.addEventListener("click", async () => {
            if (pkgState.pinBarMode === "multi") {
                const built = buildPinPayload();
                const manual = document.getElementById("selectPinRule")?.value === "manual";
                const override = manual ? built : { ...built };
                delete override.w; delete override.h;
                const pins = pkgState.selectedPins;
                const data = await applyPinsBulk(override, manual ? NaN : built.w, manual ? NaN : built.h, null, pins);
                if (data?.status === "ok") showToast(`${data.pins} selected pins modified`);
                await refreshApertureData();
                selectPkgPins(pins);
                return;
            }
            if (pkgState.pinBarMode === "all") {
                const built = buildPinPayload();
                const manual = document.getElementById("selectPinRule")?.value === "manual";
                const override = manual ? built : { rule_type: built.rule_type, undersize_w: built.undersize_w, undersize_h: built.undersize_h };
                const data = await applyPinsBulk(override, manual ? NaN : built.w, manual ? NaN : built.h);
                if (data?.status === "ok") showToast(`All ${data.pins} pins modified`);
                await refreshApertureData();
                return;
            }
            if (!pkgState.activePkgName || pkgState.selectedPin === null) return;
            const pinNum = pkgState.selectedPin;
            const payload = buildPinPayload();

            try {
                const res = await fetch(`/api/package/${encodeURIComponent(pkgState.activePkgName)}/pin/${pinNum}/override`, {
                    method: "POST",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify(payload)
                });
                const data = await res.json();
                if (data.preview) {
                    pkgState.activePackageData = data.preview;
                    populatePinDropdown(data.preview.pads, data.preview.pad_overrides);
                    const dropdown = document.getElementById("selectPinDropdown");
                    if (dropdown) dropdown.value = pinNum;
                    renderPkgPreview();
                    showToast(`Pin ${pinNum} modified!`);
                }
                await refreshApertureData();
            } catch (e) {
                console.error("Fehler beim Speichern des Pin-Overrides:", e);
            }
        });
    }

    const btnResetPadOverride = document.getElementById("btnResetPadOverride");
    // Werte des gewählten Pins auf alle Pins übertragen
    document.getElementById("btnApplyPinToAll")?.addEventListener("click", async () => {
        if (!pkgState.activePkgName || pkgState.selectedPin === null) return;
        const v = readPinBarValues();
        if (v.ruleType === "manual") {
            // Feste Öffnung auf gleich große Pads übertragen; Versatz spiegelbildlich je Seite des Bauteils
            const built = buildPinPayload();
            const pads = pkgState.activePackageData?.pads || [];
            const ref = pads.find(p => p.pin === pkgState.selectedPin);
            if (!ref) return;
            const same = pads.filter(p => Math.abs(p.w - ref.w) < 0.005 && Math.abs(p.h - ref.h) < 0.005);
            const sx = (a, b) => (Math.sign(a) || 1) === (Math.sign(b) || 1) ? 1 : -1;
            const pkgName = pkgState.activePkgName;
            let data = null;
            for (const p of same) {
                const o = { ...built, offset_x: built.offset_x * sx(p.cx, ref.cx), offset_y: built.offset_y * sx(p.cy, ref.cy) };
                data = await (await fetch(`/api/package/${encodeURIComponent(pkgName)}/pin/${p.pin}/override`, {
                    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(o)
                })).json();
            }
            if (data?.preview && pkgName === pkgState.activePkgName) {
                pkgState.activePackageData = data.preview;
                populatePinDropdown(data.preview.pads, data.preview.pad_overrides);
                const dd = document.getElementById("selectPinDropdown");
                if (dd) dd.value = pkgState.selectedPin;
                renderPkgPreview();
            }
            showToast(`Manual aperture of pin ${pkgState.selectedPin} applied to ${same.length} pins of equal size (offset mirrored)`);
            await refreshApertureData();
            return;
        }
        // Nur auf gleichartige Pads (gleiche Größe und Form) übertragen – alle Regelparameter des Pins
        const built = buildPinPayload();
        const same = samePadPins(pkgState.selectedPin);
        const src = { ...(pinOverrideOf(pkgState.selectedPin) || {}), ...built };
        delete src.w; delete src.h;
        const data = await applyPinsBulk(src, built.w, built.h, null, same);
        if (data?.status === "ok") showToast(`Settings of pin ${pkgState.selectedPin} applied to ${data.pins} identical pads`);
        await refreshApertureData();
    });

    if (btnResetPadOverride) {
        btnResetPadOverride.addEventListener("click", async () => {
            if (pkgState.pinBarMode === "multi" && pkgState.activePkgName) {
                const pins = pkgState.selectedPins;
                const d = await (await fetch(`/api/package/${encodeURIComponent(pkgState.activePkgName)}/pins/reset`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ pins }) })).json();
                if (d.preview) { pkgState.activePackageData = d.preview; populatePinDropdown(d.preview.pads, d.preview.pad_overrides); }
                await refreshApertureData();
                selectPkgPins(pins);
                showToast(`${d.pins} pins reset`);
                return;
            }
            if (pkgState.pinBarMode === "all" && pkgState.activePkgName) {
                const res = await fetch(`/api/package/${encodeURIComponent(pkgState.activePkgName)}/pins/override`, { method: "DELETE" });
                const data = await res.json();
                if (data.preview) {
                    pkgState.activePackageData = data.preview;
                    populatePinDropdown(data.preview.pads, data.preview.pad_overrides);
                    openAllPinsBar();
                    showToast("All pin modifications reset");
                }
                await refreshApertureData();
                return;
            }
            if (!pkgState.activePkgName || pkgState.selectedPin === null) return;
            const pinNum = pkgState.selectedPin;
            try {
                const res = await fetch(`/api/package/${encodeURIComponent(pkgState.activePkgName)}/pin/${pinNum}/override`, {
                    method: "DELETE"
                });
                const data = await res.json();
                if (data.preview) {
                    pkgState.activePackageData = data.preview;
                    populatePinDropdown(data.preview.pads, data.preview.pad_overrides);
                    const pad = data.preview.pads.find(p => p.pin === pinNum);
                    if (pad) selectPkgPin(pinNum, pad);
                    renderPkgPreview();
                    showToast(`Pin ${pinNum} reset to default`);
                }
                await refreshApertureData();
            } catch (e) {
                console.error("Fehler beim Reset des Pin-Overrides:", e);
            }
        });
    }

    // Single Pad Override Close
    document.getElementById("btnClosePadBar").addEventListener("click", () => {
        if (pkgState.pinBarMode === "single" || pkgState.pinBarMode === "multi") {
            openAllPinsBar(); // zurück zur Bearbeitung aller Pins
            return;
        }
        document.getElementById("singlePadBar").classList.add("hidden");
        pkgState.selectedPin = null;
        pkgState.pinBarMode = null;
        const dropdown = document.getElementById("selectPinDropdown");
        if (dropdown) dropdown.value = "all";
        renderPkgPreview();
    });
}

// -------------------------------------------------------------
// DFM Bridge Check & Pin-in-Paste (PiP) Implementation
// -------------------------------------------------------------
let currentPipData = null;

async function calculatePip() {
    const dDrill = parseFloat(document.getElementById("pipDrill")?.value) || 1.0;
    const pinShape = document.getElementById("pipPinShape")?.value || "round";
    const pinW = parseFloat(document.getElementById("pipPinW")?.value) || 0.6;
    const pinH = parseFloat(document.getElementById("pipPinH")?.value) || 0.6;
    const hPcb = parseFloat(document.getElementById("pipPcbThick")?.value) || 1.6;
    const metalRatio = parseFloat(document.getElementById("pipMetalRatio")?.value) || 50;
    const vMeniscus = parseFloat(document.getElementById("pipMeniscus")?.value) || 0.0;
    const stencilThick = parseFloat(document.getElementById("pipStencilThick")?.value) || 0.120;
    const shape = state.pipShape || "rectangle";

    try {
        const res = await fetch("/api/pip/calculate", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                d_drill: dDrill,
                pin_shape: pinShape,
                pin_w: pinW,
                pin_h: pinH,
                h_pcb: hPcb,
                metal_ratio: metalRatio / 100.0,
                v_meniscus: vMeniscus,
                stencil_thick: stencilThick,
                overprint_shape: shape
            })
        });
        const data = await res.json();
        currentPipData = data;

        const elHole = document.getElementById("pipResHoleNet");
        const elPaste = document.getElementById("pipResPasteReq");
        const elArea = document.getElementById("pipResAreaReq");
        if (elHole) elHole.innerText = `${data.v_hole_net_mm3.toFixed(3)} mm³`;
        if (elPaste) elPaste.innerText = `${data.v_paste_soll_mm3.toFixed(3)} mm³`;
        if (elArea) elArea.innerText = `${data.a_aperture_req_mm2.toFixed(2)} mm²`;

        if (data.geometries.rectangle) {
            const elR = document.getElementById("pipDimRect");
            if (elR) elR.innerText = `${data.geometries.rectangle.w.toFixed(2)} × ${data.geometries.rectangle.h.toFixed(2)} mm`;
        }
        if (data.geometries.donut) {
            const elD = document.getElementById("pipDimDonut");
            if (elD) elD.innerText = `${data.geometries.donut.d_outer.toFixed(2)} mm`;
        }
        if (data.geometries.cross) {
            const elC = document.getElementById("pipDimCross");
            if (elC) elC.innerText = `${data.geometries.cross.total_span.toFixed(2)} mm`;
        }
    } catch (e) {
        console.error("Fehler bei PiP-Berechnung:", e);
    }
}

async function applyPipToAperture() {
    if (!currentPipData || state.selectedApertureId === null) return;
    const shape = state.pipShape || "rectangle";
    const geom = currentPipData.geometries[shape];

    try {
        const res = await fetch("/api/pip/apply", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                ap_id: state.selectedApertureId,
                pip_data: geom
            })
        });
        const data = await res.json();
        if (data.status === "ok") {
            document.getElementById("pipModal")?.classList.add("hidden");
            showToast(`PiP Overprint (${shape}) applied to aperture #${state.selectedApertureId}!`);
            await refreshApertureData();
        }
    } catch (e) {
        console.error("Fehler beim Anwenden der PiP-Apertur:", e);
    }
}

async function runBridgeCheck(minBridge = null) {
    const minB = minBridge !== null ? minBridge : (parseFloat(document.getElementById("inputMinBridgeVal")?.value) || state.minBridge || 0.10);
    try {
        const res = await fetch("/api/dfm/bridge_check", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ min_bridge: minB })
        });
        const data = await res.json();
        state.bridgeViolations = data.violations || [];
        state.minBridge = data.min_bridge;
        const lblMb = document.getElementById("lblMinBridge");
        if (lblMb) lblMb.innerText = data.min_bridge.toFixed(2);
        const inMb = document.getElementById("inputMinBridgeVal");
        if (inMb) inMb.value = data.min_bridge.toFixed(2);

        updateBridgeViolationsBadge(data.count);

        const summary = document.getElementById("bridgeResultsSummary");
        if (summary) {
            if (data.count === 0) {
                summary.innerHTML = `<strong>OK:</strong> No webs below ${data.min_bridge.toFixed(2)} mm found! Stencil is ready for laser cutting.`;
                summary.style.color = "#4ade80";
            } else {
                summary.innerHTML = `<strong>${data.count} web conflicts found!</strong> Min. distance ${data.min_bridge.toFixed(2)} mm violated (collision risk in laser cutting).`;
                summary.style.color = "#f87171";
            }
        }

        const tbody = document.getElementById("bridgeViolationsTableBody");
        if (tbody) {
            if (data.count === 0) {
                tbody.innerHTML = `<tr><td colspan="4" style="text-align:center; padding: 16px; color: var(--text-secondary);">No web violations found.</td></tr>`;
            } else {
                tbody.innerHTML = data.violations.map(v => `
                    <tr style="border-bottom: 1px solid rgba(255,255,255,0.06);">
                        <td style="padding: 6px 10px;">
                            <span class="badge ${v.severity === 'CRIT' ? 'crit' : 'warn'}" style="font-size:0.7rem; padding: 2px 6px;">
                                ${v.severity === 'CRIT' ? 'Critical' : 'Warn'}
                            </span>
                        </td>
                        <td style="padding: 6px 10px; font-weight: 700; color: ${v.severity === 'CRIT' ? '#f87171' : '#fbbf24'};">
                            ${v.dist.toFixed(3)} mm
                        </td>
                        <td style="padding: 6px 10px; color: var(--text-secondary);">
                            #${v.ap_id_1} (${escapeHtml(v.pkg_1)}) &harr; #${v.ap_id_2} (${escapeHtml(v.pkg_2)})
                        </td>
                        <td style="padding: 6px 10px; text-align: right;">
                            <button class="btn btn-xs btn-secondary" data-x="${v.p1[0]}" data-y="${v.p1[1]}">
                                Go to
                            </button>
                        </td>
                    </tr>
                `).join("");
                // Ohne Inline-Handler (Content Security Policy)
                tbody.querySelectorAll("button[data-x]").forEach(b => b.addEventListener("click", () => zoomToBridgeViolation(+b.dataset.x, +b.dataset.y)));
            }
        }

        render();
    } catch (e) {
        console.error("Fehler bei Stegprüfung:", e);
    }
}

function updateBridgeViolationsBadge(count = null) {
    const badge = document.getElementById("badgeBridgeViolations");
    const num = count !== null ? count : (state.bridgeViolations ? state.bridgeViolations.length : 0);
    if (!badge) return;
    if (num > 0) {
        badge.innerText = num;
        badge.classList.remove("hidden");
        const ic = document.getElementById("bridgeWarnIcon");
        if (ic) ic.innerHTML = icon("warning");
    } else {
        badge.classList.add("hidden");
        const ic = document.getElementById("bridgeWarnIcon");
        if (ic) ic.innerHTML = icon("microscope");
    }
}

window.zoomToBridgeViolation = function(x, y) {
    document.getElementById("bridgeModal")?.classList.add("hidden");
    state.zoom = Math.max(state.zoom, 35);
    const rect = container.getBoundingClientRect();
    state.panX = rect.width / 2 - x * state.zoom;
    state.panY = rect.height / 2 - y * state.zoom;
    render();
    showToast(`Focused on web at X: ${x.toFixed(2)}, Y: ${y.toFixed(2)} mm`);
};




// -------------------------------------------------------------
// Synchronisation zwischen Fenstern (Desktop): Aufrufe aus desktop.js
// -------------------------------------------------------------
async function refreshActivePackage() {
    if (!pkgState.activePkgName) return;
    const res = await fetch(`/api/package/${encodeURIComponent(pkgState.activePkgName)}`);
    const data = await res.json();
    pkgState.activePackageData = data;
    populatePinDropdown(data.pads, data.pad_overrides);
    const dd = document.getElementById("selectPinDropdown");
    if (dd) dd.value = pkgState.selectedPin ?? "all";
    // Panel für das bisherige Ziel neu füllen, sonst schreibt die nächste Änderung alte/fremde Werte
    const t = pkgState.ruleTarget;
    if (t?.kind === "pin" && t.id === data.name) setPinRuleTarget(t.pin);
    else if (t?.kind === "pins" && t.id === data.name) selectPkgPins(t.pins);
    else if (t?.kind === "group") await selectGroupRule(t.id);
    else if (t?.kind === "zone" && (state.zones || []).some(z => z.id === t.id)) await selectZoneRule(t.id);
    else if (t?.kind === "component") await selectComponentRule(t.id);
    else setPackageRuleTarget();
    renderPkgPreview();
    renderPkgTree();
}

// Anderes Fenster hat Daten geändert → eigene Ansicht nachladen
window.scOnEngineChanged = async (info) => {
    const reloadAll = /import|load|close|upload|open_windows|project\/open/.test(info.path || "");
    refreshHistory();
    refreshSides();
    if (/\/api\/side$/.test(info.path || "")) await loadLayers();
    if (!document.body.classList.contains("view-package-only")) {
        if (reloadAll) await loadLayers();
        await refreshApertureData();
        if (reloadAll) fitToScreen();
    }
    if (typeof pkgInitialized !== "undefined" && pkgInitialized) {
        await loadPackagesList();
        if (reloadAll) {
            pkgState.activePkgName = null;
            pkgState.activePackageData = null;
        } else {
            await refreshActivePackage();
        }
    }
};

// Board-Fenster fordert ein Package (und Pin) im Generator-Fenster an
window.scOpenPackage = async ({ pkg, pin, rot }) => {
    if (currentWorkspaceMode !== "package") switchWorkspace("package", pkg, rot || 0);
    pkgState.rotation = rot || 0;
    await selectPackage(pkg);
    if (pin !== null && pin !== undefined) {
        const pad = pkgState.activePackageData?.pads?.find(p => p.pin === pin);
        if (pad) selectPkgPin(pin, pad);
    }
};


// -------------------------------------------------------------
// Projekt speichern/laden, Undo/Redo, ungespeicherte Änderungen
// -------------------------------------------------------------
const projectState = { history: { dirty: false, can_undo: false, can_redo: false } };

// Jede ändernde API-Antwort liefert den Bearbeitungsstand mit → Anzeige aktualisieren
(function hookHistoryFromResponses() {
    const baseFetch = window.fetch.bind(window);
    window.fetch = async (input, opts = {}) => {
        const res = await baseFetch(input, opts);
        try {
            const url = typeof input === "string" ? input : input?.url;
            const m = (opts.method || "GET").toUpperCase();
            if (url && url.startsWith("/api/") && m !== "GET") {
                res.clone().json().then(d => { if (d && d.history) updateHistoryUI(d.history); }).catch(() => {});
            }
        } catch (e) { /* Anzeige ist optional */ }
        return res;
    };
})();

function updateHistoryUI(h) {
    if (!h) return;
    projectState.history = h;
    const undo = document.getElementById("btnUndo"), redo = document.getElementById("btnRedo");
    if (undo) { undo.disabled = !h.can_undo; undo.title = h.can_undo ? `Undo: ${h.undo_label} (Ctrl+Z)` : "Undo (Ctrl+Z)"; }
    if (redo) { redo.disabled = !h.can_redo; redo.title = h.can_redo ? `Redo: ${h.redo_label} (Ctrl+Y)` : "Redo (Ctrl+Y)"; }
    const lbl = document.getElementById("lblProjectName");
    if (lbl) {
        lbl.textContent = h.project_name || (h.dataset_loaded ? "Untitled Project" : "–");
        lbl.title = h.project_path || "Not saved yet";
        lbl.classList.toggle("dirty", Boolean(h.dirty));
    }
    const base = document.body.classList.contains("view-package-only") ? "Stencil Generator" : "SMT StencilCraft";
    document.title = `${h.dirty ? "● " : ""}${h.project_name ? h.project_name + " – " : ""}${base}`;
}

async function refreshHistory() {
    try {
        const r = await fetch("/api/history");
        updateHistoryUI(await r.json());
        // Nach jeder Änderung (auch Rückgängig) die Status-Farben der Package-Liste nachziehen
        if (pkgState.packages.length) loadPackagesList();
    } catch (e) { /* ignorieren */ }
}

// Auswahl-Dialog: liefert den "value" des geklickten Buttons (ESC/Hintergrund = "cancel")
// Bauteilpins ohne Kundenpaste: PCB Investigator erzeugt dort Öffnungen aus dem Kupfer
async function offerMissingPaste(explicit) {
    const m = await (await fetch("/api/paste/missing")).json();
    if (!m.count) {
        if (explicit) showToast("All component pins have a paste aperture.");
        return 0;
    }
    const top = Object.entries(m.packages || {}).sort((a, b) => b[1] - a[1]).slice(0, 8).map(([p, n]) => `${p}: ${n}`).join(", ");
    const r = await askChoice("Pins without Customer Paste",
        `${m.count} component pins have no aperture in the customer paste layer (${top}).

` +
        "With \"Package Group\", PCB Investigator creates apertures from copper there. Add them? The rules then decide per package (\"Take Existing\"/\"Ignore\" → no aperture). Ctrl+Z to undo.",
        [{ label: "Don't add", value: "no", cls: "btn-ghost" }, { label: `Add ${m.count}`, value: "yes", cls: "btn-primary" }]);
    if (r !== "yes") return 0;
    const d = await (await fetch("/api/paste/add_missing", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" })).json();
    await refreshApertureData();
    showToast(`✓ ${d.added} apertures added from copper`);
    return d.added;
}

async function importPcbiLibrary() {
    const cust = pkgState.currentCustomer || "";
    const how = await askChoice("PCB-Investigator Customer Library",
        "Apply rules (packages, pins, groups) from a PCB-I file ruleTreeFile_*.xml to the dataset.\n\n" +
        `Also save to StencilCraft library "${cust || "Standard"}" (incl. packages not in this job)?`,
        [{ label: "Cancel", value: "cancel", cls: "btn-ghost" },
         { label: "Apply Only", value: "apply" },
         { label: "Apply + Save", value: "save", cls: "btn-primary" }]);
    if (how === "cancel") return;
    const res = await fetch("/api/pcbi/import", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ customer: how === "save" ? (cust || "Standard") : "", generate: true })
    });
    const d = await res.json();
    if (d.status === "cancelled") return;
    if (!res.ok || d.status !== "ok") { showToast(`${d.message || d.error || "Import failed"}`); return; }
    await loadPackagesList();
    await refreshApertureData();
    const missing = d.missing_in_library || [];
    let msg = `${d.matched.length} of ${d.library_packages} library packages found in dataset, ${d.pin_rules} pin rules, groups: ${(d.group_rules || []).join(", ")}.`;
    msg += `
${d.modified_count ?? 0} apertures recalculated.`;
    if (missing.length) msg += `

No library entry (${missing.length}) – inherit from their group:
${missing.slice(0, 15).map(p => p.name).join(", ")}${missing.length > 15 ? " …" : ""}`;
    if (d.not_in_dataset?.length) msg += `

In library but not in job: ${d.not_in_dataset.length}`;
    const unknown = Object.keys(d.unknown_rules || {});
    if (unknown.length) msg += `

Unknown PCB-I scripts (treated as "Take Existing"): ${unknown.join(", ")}`;
    if (d.warnings?.length) msg += `

${d.warnings.join("\n")}`;
    if (d.library_file) msg += `

Saved to: ${d.library_file}`;
    await askChoice("PCB-I library applied", msg, [{ label: "OK", value: "ok", cls: "btn-primary" }]);
    if (d.missing_paste > 0) await offerMissingPaste(false);
}

function askChoice(title, text, choices) {
    return new Promise(resolve => {
        const modal = document.getElementById("choiceModal");
        document.getElementById("choiceTitle").textContent = title;
        document.getElementById("choiceText").textContent = text;
        const box = document.getElementById("choiceButtons");
        box.innerHTML = "";
        const done = (v) => { modal.classList.add("hidden"); window.removeEventListener("keydown", onKey, true); resolve(v); };
        const onKey = (e) => { if (e.key === "Escape") { e.stopPropagation(); done("cancel"); } };
        for (const c of choices) {
            const b = document.createElement("button");
            b.className = `btn ${c.cls || "btn-secondary"}`;
            b.textContent = c.label;
            b.addEventListener("click", () => done(c.value));
            box.appendChild(b);
        }
        window.addEventListener("keydown", onKey, true);
        modal.classList.remove("hidden");
        box.lastChild?.focus();
    });
}

// Vor verlustbehafteten Aktionen: bei ungespeicherten Änderungen speichern/verwerfen/abbrechen
async function guardUnsaved(actionText) {
    await refreshHistory();
    if (!projectState.history.dirty) return true;
    const r = await askChoice("Unsaved Changes",
        `The project has unsaved changes.\nSave before ${actionText}?`,
        [{ label: "Cancel", value: "cancel", cls: "btn-ghost" },
         { label: "Discard", value: "discard", cls: "btn-danger" },
         { label: "Save", value: "save", cls: "btn-primary" }]);
    if (r === "cancel") return false;
    if (r === "save") return saveProject(false);
    return true;
}
window.guardUnsaved = guardUnsaved;

async function saveProject(saveAs = false) {
    const res = await fetch("/api/project/save", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ save_as: saveAs })
    });
    const d = await res.json();
    if (d.status === "ok") { showToast(`Project saved: ${d.path}`); return true; }
    if (d.status !== "cancelled") showToast(`Save failed: ${d.message}`);
    return false;
}
window.saveProject = saveProject;

async function reloadAllViews(full = false) {
    if (full) { await loadLayers(); pkgState.libAsked = false; pkgState.libEdited = false; }
    refreshSides();
    await refreshApertureData();
    if (full) fitToScreen();
    if (typeof pkgInitialized !== "undefined" && pkgInitialized) {
        await loadPackagesList();
        if (full) {
            pkgState.activePkgName = null;
            if (currentWorkspaceMode === "package") await initPackageMode();
        } else if (pkgState.activePkgName) {
            await refreshActivePackage();
        }
    }
    await refreshHistory();
}

async function doUndo() {
    const d = await (await fetch("/api/undo", { method: "POST" })).json();
    if (d.status === "ok") { showToast(`↶ Undo: ${d.label}`, 2000); await reloadAllViews(); }
}

async function doRedo() {
    const d = await (await fetch("/api/redo", { method: "POST" })).json();
    if (d.status === "ok") { showToast(`↷ Redo: ${d.label}`, 2000); await reloadAllViews(); }
}

async function openProject(path = null, extra = {}) {
    if (!extra.relocate && !extra.recovered && !(await guardUnsaved("opening another project"))) return;
    const res = await fetch("/api/project/open", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path, ...extra })
    });
    const d = await res.json();
    if (d.status === "missing_dataset") {
        const r = await askChoice("Dataset Not Found",
            `The project dataset is no longer at the saved location:\n${(d.dataset_paths || []).join("\n")}\n\nPlease select the dataset (ODB++ / Gerber) again.`,
            [{ label: "Cancel", value: "cancel", cls: "btn-ghost" }, { label: "Select Dataset…", value: "relocate", cls: "btn-primary" }]);
        // "recovered" mitgeben, damit die wiederhergestellte Sitzung nicht als gespeichert gilt
        if (r === "relocate") return openProject(d.path, { ...extra, relocate: true });
        return false;
    }
    if (d.status === "ok") {
        await reloadAllViews(true);
        showToast(extra.recovered ? "Session restored" : `Project opened: ${d.path}`);
        if (d.warnings?.length) setTimeout(() => showToast(`${d.warnings.join(" ")}`, 6000), 1200);
        return true;
    } else if (d.status !== "cancelled") {
        showToast(`Could not open project: ${d.message}`, 6000);
    }
    return false;
}

async function renderRecentProjects() {
    const box = document.getElementById("recentProjectsList");
    if (!box) return;
    const d = await (await fetch("/api/project/recent")).json();
    box.innerHTML = "";
    if (!d.projects?.length) { box.innerHTML = '<div class="dropdown-empty">No projects yet</div>'; return; }
    for (const p of d.projects) {
        const b = document.createElement("button");
        b.textContent = p.split(/[\\/]/).pop();
        b.title = p;
        b.addEventListener("click", () => { closeProjectMenu(); openProject(p); });
        box.appendChild(b);
    }
}

function closeProjectMenu() { document.getElementById("projectMenuList")?.classList.add("hidden"); }

function setupProjectUI() {
    document.getElementById("btnUndo")?.addEventListener("click", doUndo);
    document.getElementById("btnRedo")?.addEventListener("click", doRedo);
    const menu = document.getElementById("projectMenuList");
    document.getElementById("btnProjectMenu")?.addEventListener("click", async (e) => {
        e.stopPropagation();
        const willOpen = menu.classList.contains("hidden");
        menu.classList.toggle("hidden");
        if (willOpen) await renderRecentProjects();
    });
    document.addEventListener("click", (e) => { if (!e.target.closest("#projectMenu")) closeProjectMenu(); });
    menu?.querySelectorAll("button[data-act]").forEach(b => b.addEventListener("click", () => {
        closeProjectMenu();
        if (b.dataset.act === "open") openProject();
        else if (b.dataset.act === "save") saveProject(false);
        else if (b.dataset.act === "saveas") saveProject(true);
    }));
}

// Nach Absturz: automatische Sicherung anbieten
async function offerRecovery() {
    try {
        const info = await (await fetch("/api/project/recovery")).json();
        if (!info.exists) return;
        const r = await askChoice("Restore Session?",
            `An automatic backup with unsaved changes exists:\n${info.dataset || "Untitled"}\n\nRestore it?`,
            [{ label: "Discard", value: "discard", cls: "btn-ghost" }, { label: "Restore", value: "restore", cls: "btn-primary" }]);
        if (r === "restore") {
            // Sicherung nur verwerfen, wenn die Wiederherstellung geklappt hat (sonst ginge sie verloren)
            if (!(await openProject(info.path, { recovered: true }))) return;
            // Sicherung ist nun Teil dieser Sitzung (eigene Sicherung übernimmt)
            await fetch("/api/project/discard_recovery", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path: info.path }) });
        } else if (r === "discard") {
            await fetch("/api/project/discard_recovery", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path: info.path }) });
        }
    } catch (e) { /* ohne Sicherung weiter */ }
}

// Desktop: Fenster schließen mit ungespeicherten Änderungen
window.scHandleCloseRequest = async () => {
    if (await guardUnsaved("exiting the application")) {
        await fetch("/api/project/discard_recovery", { method: "POST" });
        window.__TAURI__?.core.invoke("confirm_close");
    }
};


// -------------------------------------------------------------
// TOP / BOT
// -------------------------------------------------------------
async function refreshSides() {
    try {
        const d = await (await fetch("/api/sides")).json();
        for (const sd of d.sides) {
            // Kopfleiste und Package-Liste zeigen dieselbe Seite
            for (const btn of document.querySelectorAll(`.side-switch button[data-side="${sd.side}"]`)) {
                const inList = btn.closest("#sideSwitch");
                btn.classList.toggle("active", sd.active && !(inList && pkgState.sideFilter === "both"));
                const usable = sd.loaded || sd.available;
                btn.disabled = !usable && !sd.active;
                const cnt = btn.querySelector(".side-count");
                if (cnt) cnt.textContent = usable ? `(${sd.apertures})` : "";
                btn.title = sd.loaded ? `${sd.side.toUpperCase()}: ${sd.apertures} paste apertures`
                    : sd.available ? `${sd.side.toUpperCase()}: no customer paste – apertures can be added from copper`
                    : `${sd.side.toUpperCase()}: no data for this side`;
            }
        }
        state.side = d.active;
    } catch (e) { /* Web-Version ohne Seitenwechsel */ }
}

async function switchSide(side, opts = {}) {
    if (side === state.side) return;
    const res = await fetch("/api/side", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ side, keep_layers: true }) });
    if (!res.ok) return;
    setPackageHighlight(null);
    state.selectedApertureId = null;
    state.selectedAperture = null;
    // Zonen-IDs gelten je Seite (beide Seiten haben "zone_1"): Zonenziel nicht mitnehmen
    if (pkgState.ruleTarget?.kind === "zone") pkgState.ruleTarget = { kind: "package", id: pkgState.activePkgName };
    await loadLayers();
    await reloadAllViews(false);
    await refreshSides();
    if (opts.silent) return;
    // Seite ohne Kundenpaste: Öffnungen aus dem Kupfer anbieten
    const sd = (await (await fetch("/api/sides")).json()).sides.find(x => x.side === side);
    if (sd && !sd.loaded) await offerMissingPaste(true);
}
window.switchSide = switchSide;

document.querySelectorAll("#boardSideSwitch button").forEach(b => b.addEventListener("click", async () => {
    if (b.dataset.side === state.side) return;
    if (pkgState.sideFilter !== "both") pkgState.sideFilter = b.dataset.side;
    await switchSide(b.dataset.side);
    renderPkgTree();
}));

document.querySelectorAll("#sideSwitch button").forEach(b => b.addEventListener("click", async () => {
    pkgState.sideFilter = b.dataset.side;
    document.querySelector('#sideSwitch button[data-side="both"]').classList.toggle("active", b.dataset.side === "both");
    if (b.dataset.side !== "both" && b.dataset.side !== state.side) await switchSide(b.dataset.side);
    await refreshSides();
    renderPkgTree();
}));

// -------------------------------------------------------------
// Export-Dialog
// -------------------------------------------------------------
const exportState = { job: "Schablone", sides: null, lastDir: null, lastReport: null };

function exportOptionsFromForm() {
    return {
        target_dir: document.getElementById("expDir").value.trim(),
        pattern: document.getElementById("expPattern").value.trim() || "{job}_{side}_{dicke}um",
        sides: document.getElementById("expSides").value,
        zones: document.getElementById("expZones").checked,
        outline: document.getElementById("expOutline").checked,
        report: document.getElementById("expReport").checked,
        x2: document.getElementById("expX2").checked
    };
}

function updateExportPreview() {
    const o = exportOptionsFromForm();
    const today = new Date().toISOString().slice(0, 10);
    const thick = Math.round((state.stats?.base_thickness || 0.12) * 1000);
    const active = exportState.sides?.active || "top";
    const loaded = (exportState.sides?.sides || []).filter(x => x.loaded).map(x => x.side);
    const sides = o.sides === "both" ? loaded : (o.sides === "active" ? [active] : [o.sides]);
    const name = (side) => o.pattern.replaceAll("{job}", exportState.job).replaceAll("{side}", side === "Kontur" ? side : side.toUpperCase()).replaceAll("{dicke}", thick).replaceAll("{datum}", today);
    const lines = [];
    for (const sd of sides) {
        lines.push(`${name(sd)}.gbr`);
        if (o.zones) lines.push(`${name(sd)}_zonen.gbr (if zones exist)`);
    }
    if (o.outline) lines.push(`${name("Kontur")}.gbr`);
    if (o.report) lines.push(`${exportState.job}_Pruefbericht_${today}.html`);
    document.getElementById("expPreview").innerHTML = lines.map(l => `<div>${escapeHtml(l)}</div>`).join("") || "–";
}

async function openExportDialog() {
    const d = await (await fetch("/api/export/settings")).json().catch(() => null);
    if (d && d.settings) {
        const st = d.settings;
        exportState.job = d.job;
        exportState.sides = d.sides;
        document.getElementById("expDir").value = st.target_dir || "";
        document.getElementById("expDir").placeholder = d.default_dir;
        document.getElementById("expPattern").value = st.pattern;
        // Eine Ansicht: immer beide Seiten (BOT ungespiegelt, der Hersteller spiegelt)
        const loadedSides = (d.sides?.sides || []).filter(x => x.loaded).map(x => x.side);
        document.getElementById("expSides").value = loadedSides.length > 1 ? (st.sides === "top" || st.sides === "bot" ? st.sides : "both") : (loadedSides[0] || "top");
        document.getElementById("expZones").checked = st.zones;
        document.getElementById("expOutline").checked = st.outline;
        document.getElementById("expReport").checked = st.report;
        document.getElementById("expX2").checked = st.x2;
        const bothLoaded = (d.sides?.sides || []).filter(x => x.loaded).length > 1;
        document.querySelector('#expSides option[value="both"]').disabled = !bothLoaded;
        for (const sd of d.sides?.sides || []) {
            const opt = document.querySelector(`#expSides option[value="${sd.side}"]`);
            if (opt) opt.disabled = !sd.loaded;
        }
    }
    document.getElementById("exportForm").classList.remove("hidden");
    document.getElementById("exportResult").classList.add("hidden");
    document.getElementById("btnExpRun").classList.remove("hidden");
    document.getElementById("btnExpOpenDir").classList.add("hidden");
    document.getElementById("btnExpOpenReport").classList.add("hidden");
    updateExportPreview();
    document.getElementById("exportModal").classList.remove("hidden");
}
window.openExportDialog = openExportDialog;

function setupExportDialog() {
    ["expPattern", "expSides", "expZones", "expOutline", "expReport", "expX2", "expDir"].forEach(id => {
        const el = document.getElementById(id);
        el?.addEventListener("input", updateExportPreview);
        el?.addEventListener("change", updateExportPreview);
    });
    document.getElementById("btnCloseExportModal")?.addEventListener("click", () => document.getElementById("exportModal").classList.add("hidden"));
    document.getElementById("btnExpPickDir")?.addEventListener("click", async () => {
        const d = await (await fetch("/api/export/pick_dir", { method: "POST" })).json();
        if (d.status === "ok") { document.getElementById("expDir").value = d.path; updateExportPreview(); }
    });
    const openPath = (p) => p && window.__TAURI__?.core.invoke("open_path", { path: p }).catch(e => showToast(`${e}`));
    document.getElementById("btnExpOpenDir")?.addEventListener("click", () => openPath(exportState.lastDir));
    document.getElementById("btnExpOpenReport")?.addEventListener("click", () => openPath(exportState.lastReport));
    document.getElementById("btnExpRun")?.addEventListener("click", async () => {
        const btn = document.getElementById("btnExpRun");
        btn.disabled = true;
        try {
            const res = await fetch("/api/export", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ options: exportOptionsFromForm() }) });
            const d = await res.json();
            if (d.status !== "ok") { showToast(`Export failed: ${d.message}`, 6000); return; }
            exportState.lastDir = d.export_dir;
            exportState.lastReport = d.report_file;
            const icon = { stencil: "", zones: "", outline: "", report: "" };
            document.getElementById("exportMsg").textContent = d.message;
            document.getElementById("exportFileList").innerHTML = d.files.map(f => `<div>${icon[f.kind] || ""} ${escapeHtml(f.path)}${f.count !== undefined ? ` (${f.count})` : ""}</div>`).join("");
            document.getElementById("exportForm").classList.add("hidden");
            document.getElementById("exportResult").classList.remove("hidden");
            btn.classList.add("hidden");
            const desktop = Boolean(window.__TAURI__);
            document.getElementById("btnExpOpenDir").classList.toggle("hidden", !desktop);
            document.getElementById("btnExpOpenReport").classList.toggle("hidden", !desktop || !d.report_file);
            showToast("Export complete");
        } finally {
            btn.disabled = false;
        }
    });
}


// -------------------------------------------------------------
// Regelziele: Gruppe / Zone / Bauteil (PCB Investigator: Package-, Zone-, Component-Rules)
// -------------------------------------------------------------
const GROUP_LABELS = { SMD: "SMD Packages", THT: "THT Packages", Other: "Other Packages", Fiducial: "Fiducials", Library: "Library" };

function updateRuleSourceBanner() {
    const banner = document.getElementById("ruleSourceBanner");
    const text = document.getElementById("ruleSourceText");
    const btn = document.getElementById("btnRemoveTargetRule");
    if (!banner) return;
    const t = pkgState.ruleTarget || { kind: "package" };
    banner.classList.remove("hidden", "inherited", "own");
    btn.classList.add("hidden");
    const inheritOpt = document.querySelector('#selectPkgRule option[value="inherit"]');
    if (inheritOpt) {
        inheritOpt.disabled = t.kind === "group";
        // Wie PCB-I: "Inherit    SMD Packages -> Handle Stencil Creation by Package Group"
        const d = pkgState.activePackageData;
        const effLabel = document.querySelector(`#selectPkgRule option[value="${d?.effective_rule?.rule_type}"]`)?.textContent;
        inheritOpt.textContent = (t.kind === "pin" || t.kind === "pins") ? "Like Package (no own pin rule)"
            : (t.kind === "package" && d?.rule?.rule_type === "inherit" && effLabel) ? `Inherit    ${GROUP_LABELS[d.group] || d.group || "SMD Packages"} -> ${effLabel}`
            : "Inherit – use group rule";
    }
    banner.classList.toggle("pin-target", t.kind === "pin" || t.kind === "pins");
    if (t.kind === "pins") {
        const own = (t.pins || []).filter(p => { const o = pinOverrideOf(p); return o && o.rule_type && o.rule_type !== "inherit"; }).length;
        banner.classList.add(own ? "own" : "inherited");
        text.textContent = `● Selection: ${t.pins.length} pins – changes here apply only to these pins (${own} with own rule). "Like Package" resets the selection.`;
        if (own) btn.classList.remove("hidden");
        return;
    }
    if (t.kind === "pin") {
        const ov = pinOverrideOf(t.pin);
        if (ov && ov.rule_type && ov.rule_type !== "inherit") {
            banner.classList.add("own");
            text.textContent = `● Pin ${t.pin} only: own rule (${ov.manual ? "manual" : ov.rule_type}) – other pins unchanged. Select "Like Package" to reset the pin.`;
            btn.classList.remove("hidden");
        } else {
            banner.classList.add("inherited");
            text.textContent = `↳ Pin ${t.pin} follows the package rule – changes here apply only to this pin. For all pins select "All Pins" above.`;
        }
        return;
    }
    if (t.kind === "package") {
        const d = pkgState.activePackageData;
        if (d?.rule?.rule_type === "inherit") {
            banner.classList.add("inherited");
            text.textContent = `↳ Rule inherited from ${d.rule_source} – changing a parameter creates an own package rule`;
        } else {
            banner.classList.add("own");
            text.textContent = `● Own package rule (${d?.rule?.rule_type || "-"}) – select "Inherit" to use the group rule`;
        }
    } else if (t.kind === "group") {
        text.textContent = `Group rule "${GROUP_LABELS[t.id] || t.id}" – applies to all packages of this group set to "Inherit"`;
    } else if (t.kind === "component") {
        const c = pkgState.activeComponent;
        if (c?.has_rule) {
            banner.classList.add("own");
            text.textContent = `● Component rule ${t.id} (overrides zone, package and group)`;
            btn.classList.remove("hidden");
        } else {
            banner.classList.add("inherited");
            text.textContent = `↳ ${t.id} uses ${c?.rule_source || "-"} – changing a parameter creates a component rule`;
        }
    } else if (t.kind === "zone") {
        const z = (state.zones || []).find(x => x.id === t.id);
        if (z?.rule) {
            banner.classList.add("own");
            text.textContent = `● Zone rule "${z.name}" (applies to apertures in the zone, except component rules)`;
            btn.classList.remove("hidden");
        } else {
            banner.classList.add("inherited");
            text.textContent = `Zone "${z?.name || t.id}" has no rule yet – changing a parameter creates it`;
        }
    }
}

function drawPkgMessage(lines) {
    if (!pkgCtx || !pkgCanvas) return;
    const r = pkgCanvas.getBoundingClientRect();
    pkgCtx.save();
    pkgCtx.setTransform(1, 0, 0, 1, 0, 0);
    pkgCtx.fillStyle = "#000";
    pkgCtx.fillRect(0, 0, pkgCanvas.width, pkgCanvas.height);
    resetToCssPixels(pkgCtx);
    pkgCtx.fillStyle = "#e2e8f0";
    pkgCtx.textAlign = "center";
    pkgCtx.font = "600 16px Inter, sans-serif";
    (Array.isArray(lines) ? lines : [lines]).forEach((l, i) => {
        if (i === 1) { pkgCtx.font = "13px Inter, sans-serif"; pkgCtx.fillStyle = "#94a3b8"; }
        pkgCtx.fillText(l, r.width / 2, r.height / 2 - 20 + i * 24);
    });
    pkgCtx.restore();
}

function groupMessage(key) {
    const g = (pkgState.groups || []).find(x => x.key === key);
    return [`Group rule: ${GROUP_LABELS[key] || key}`,
            g ? `${g.inheriting} of ${g.packages} packages inherit this rule · ${g.pads} pads on the active side` : "",
            "\"Generate Stencil Pads\" applies it to all inheriting pads."];
}

async function selectGroupRule(key) {
    const d = await (await fetch("/api/groups")).json();
    pkgState.groups = d.groups;
    const g = d.groups.find(x => x.key === key);
    if (!g) return;
    pkgState.ruleTarget = { kind: "group", id: key };
    pkgState.activePkgName = null;
    pkgState.activePackageData = null;
    pkgState.effectiveRuleType = g.rule.rule_type;
    document.getElementById("singlePadBar")?.classList.add("hidden");
    document.getElementById("canvasPkgTitle").textContent = GROUP_LABELS[key];
    document.getElementById("canvasPkgPitch").textContent = "";
    populateRuleInputs(g.rule);
    updateRuleSourceBanner();
    renderPkgTree();
    drawPkgMessage(groupMessage(key));
}

function switchGenTab(tab) {
    document.querySelectorAll("#genTabs button").forEach(b => b.classList.toggle("active", b.dataset.tab === tab));
    document.getElementById("genTabPackages")?.classList.toggle("hidden", tab !== "packages");
    document.getElementById("genTabZones")?.classList.toggle("hidden", tab !== "zones");
    document.getElementById("genTabComponents")?.classList.toggle("hidden", tab !== "components");
    if (tab === "zones") renderZoneRuleList();
    if (tab === "components") loadComponentList();
}

function renderZoneRuleList() {
    const box = document.getElementById("zoneRuleList");
    if (!box) return;
    const zones = state.zones || [];
    box.innerHTML = zones.length ? "" : '<div class="dropdown-empty" style="padding:12px">No zones on this side. Draw a rule zone on the board or create a step zone.</div>';
    for (const z of zones) {
        const el = document.createElement("div");
        const active = pkgState.ruleTarget?.kind === "zone" && pkgState.ruleTarget.id === z.id;
        el.className = `list-item ${active ? "active" : ""}`;
        el.innerHTML = `<div class="li-main">${escapeHtml(z.name)}<div class="li-sub">${z.width.toFixed(1)} × ${z.height.toFixed(1)} mm</div></div>` +
            (z.step ? `<span class="li-badge step">${Math.round(z.thickness * 1000)} µm</span>` : "") +
            (z.rule ? `<span class="li-badge rule">${z.rule.rule_type}</span>` : '<span class="li-badge">no rule</span>');
        el.addEventListener("click", () => selectZoneRule(z.id));
        box.appendChild(el);
    }
}

async function selectZoneRule(id) {
    await refreshApertureData();
    const z = (state.zones || []).find(x => x.id === id);
    if (!z) return;
    pkgState.ruleTarget = { kind: "zone", id };
    pkgState.activePkgName = null;
    pkgState.activePackageData = null;
    const groups = (await (await fetch("/api/groups")).json()).groups;
    const base = z.rule || groups.find(g => g.key === "SMD")?.rule;
    pkgState.effectiveRuleType = base?.rule_type;
    document.getElementById("singlePadBar")?.classList.add("hidden");
    document.getElementById("canvasPkgTitle").textContent = z.name;
    document.getElementById("canvasPkgPitch").textContent = "";
    populateRuleInputs(base);
    updateRuleSourceBanner();
    renderZoneRuleList();
    const n = (state.apertures || []).filter(a => a.cx >= z.x0 && a.cx <= z.x1 && a.cy >= z.y0 && a.cy <= z.y1).length;
    drawPkgMessage([`Zone: ${z.name}`, `${n} pads in the zone${z.step ? ` · step ${Math.round(z.thickness * 1000)} µm` : " · rule zone only"}`, "\"Generate Stencil Pads\" applies the zone rule."]);
    // Im Board-Fenster die Zone zeigen
    if (window.scBroadcast) window.scBroadcast("pkg-focus", { pkg: null, pin: null });
}

async function loadComponentList() {
    const d = await (await fetch("/api/components")).json();
    pkgState.components = d.components || [];
    renderComponentList();
}

function renderComponentList() {
    const box = document.getElementById("componentList");
    if (!box) return;
    const q = (document.getElementById("compSearchInput")?.value || "").toLowerCase();
    const onlyRule = pkgState.compFilter === "rule";
    const list = (pkgState.components || []).filter(c =>
        (!q || c.refdes.toLowerCase().includes(q) || c.package.toLowerCase().includes(q)) && (!onlyRule || c.has_rule));
    box.innerHTML = list.length ? "" : '<div class="dropdown-empty" style="padding:12px">No components (only for ODB++ with component data).</div>';
    for (const c of list.slice(0, 800)) {
        const el = document.createElement("div");
        const active = pkgState.ruleTarget?.kind === "component" && pkgState.ruleTarget.id === c.refdes;
        el.className = `list-item ${active ? "active" : ""}`;
        el.innerHTML = `<div class="li-main"><b>${escapeHtml(c.refdes)}</b> <span class="li-sub">${escapeHtml(c.package)}</span></div><span class="li-badge">${c.pads} Pads</span>${c.has_rule ? '<span class="li-badge rule">own rule</span>' : ""}`;
        el.addEventListener("click", () => selectComponentRule(c.refdes));
        box.appendChild(el);
    }
}

async function selectComponentRule(refdes) {
    const d = await (await fetch(`/api/component/${encodeURIComponent(refdes)}`)).json();
    pkgState.ruleTarget = { kind: "component", id: refdes };
    pkgState.activeComponent = d;
    pkgState.activePkgName = null;
    pkgState.activePackageData = d.preview;
    pkgState.effectiveRuleType = d.effective_rule?.rule_type;
    pkgState.rotation = d.rot || 0;
    document.getElementById("singlePadBar")?.classList.add("hidden");
    document.getElementById("canvasPkgTitle").textContent = `${refdes} (${d.package})`;
    document.getElementById("canvasPkgPitch").textContent = `${d.pads} Pads`;
    populateRuleInputs(d.rule || { ...d.effective_rule, rule_type: "inherit" });
    updateRuleSourceBanner();
    autoFitPackagePreview(d.preview);
    renderPkgPreview();
    renderComponentList();
    window.scBroadcast?.("pkg-focus", { pkg: d.package, pin: null });
    state.highlightPkg = d.package;
}

function setupRuleTargets() {
    document.querySelectorAll("#genTabs button").forEach(b => b.addEventListener("click", () => switchGenTab(b.dataset.tab)));
    document.getElementById("compSearchInput")?.addEventListener("input", renderComponentList);
    document.querySelectorAll(".comp-filter").forEach(b => b.addEventListener("click", () => {
        document.querySelectorAll(".comp-filter").forEach(x => x.classList.toggle("active", x === b));
        pkgState.compFilter = b.dataset.f;
        renderComponentList();
    }));
    document.getElementById("btnNewRuleZone")?.addEventListener("click", () => {
        if (document.body.classList.contains("view-package-only")) {
            showToast("Draw a rule zone in the board window (tool \"Zone\", uncheck \"Step Zone\").", 5000);
            return;
        }
        state.drawRuleZone = true;
        switchWorkspace("board");
        startDrawingZone();
        showToast("Draw rectangle for the rule zone on the board", 3000);
    });
    document.getElementById("btnRemoveTargetRule")?.addEventListener("click", async () => {
        const t = pkgState.ruleTarget;
        if (t?.kind === "pin") { await savePinRule(t, { rule_type: "inherit" }); return; }
        if (t?.kind === "pins") { await savePinsRule(t, { rule_type: "inherit" }); return; }
        if (!t || (t.kind !== "component" && t.kind !== "zone")) return;
        await fetch(`/api/${t.kind}/${encodeURIComponent(t.id)}/rule`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ enabled: false }) });
        await fetch(`/api/${t.kind}/${encodeURIComponent(t.id)}/generate`, { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
        await refreshApertureData();
        if (t.kind === "component") { await loadComponentList(); await selectComponentRule(t.id); }
        else { renderZoneRuleList(); await selectZoneRule(t.id); }
        showToast("Own rule removed – apertures recalculated");
    });
}

// Info-Layer: AR, Fläche und Volumen je Pastenöffnung + Summe
function renderPasteInfo(data) {
    const pitchEl = document.getElementById("canvasPkgPitch");
    if (!data?.paste?.length) return;
    const T = data.base_thickness || state.stats?.base_thickness || 0.12;
    const minAr = state.stats?.ipc_min_ar || 0.66;
    let sumVol = 0, arMin = Infinity;
    const infos = data.paste.map(sl => {
        let area, perim;
        if (sl.shape === "circle") { area = Math.PI * (sl.w / 2) ** 2; perim = Math.PI * sl.w; }
        else if (sl.pts && sl.pts.length >= 3) {
            area = 0; perim = 0;
            for (let i = 0; i < sl.pts.length; i++) {
                const a = sl.pts[i], b = sl.pts[(i + 1) % sl.pts.length];
                area += a[0] * b[1] - b[0] * a[1];
                perim += Math.hypot(b[0] - a[0], b[1] - a[1]);
            }
            area = Math.abs(area) / 2;
        } else { area = sl.w * sl.h; perim = 2 * (sl.w + sl.h); }
        const ar = area / (perim * T);
        const vol = area * T * 1000; // nl
        sumVol += vol;
        arMin = Math.min(arMin, ar);
        return { sl, area, ar, vol };
    });
    if (pitchEl && pkgState.ruleTarget?.kind !== "group") {
        const base = data.pitch > 0 ? `Pitch: ${data.pitch} mm · ` : "";
        pitchEl.textContent = `${base}Paste Σ ${sumVol.toFixed(1)} nl · AR min ${arMin.toFixed(2)} · ${Math.round(T * 1000)} µm`;
    }
    if (!pkgState.showInfo) return;
    // Pin-in-Paste: Soll-Fläche je Pin aus Bohrung und Regelparametern (wie Engine)
    const er = data.effective_rule || {};
    const pipNeed = (pin) => {
        const pad = data.pads?.find(p => p.pin === pin);
        if (!pad?.drill) return null;
        const dr = pad.drill, pinD = Math.max(0.1, dr - (er.pip_pin_clearance ?? 0.25));
        const vHole = Math.max(0, (Math.PI * dr * dr / 4 - Math.PI * pinD * pinD / 4) * (er.pip_pcb_thickness ?? 1.6));
        return vHole * 1.2 / Math.min(1, Math.max(0.1, (er.pip_metal_pct ?? 50) / 100)) / T;
    };
    pkgCtx.save();
    resetToCssPixels(pkgCtx);
    pkgCtx.textAlign = "center";
    pkgCtx.textBaseline = "middle";
    for (const inf of infos) {
        const sw = Math.min(inf.sl.w, inf.sl.h) * pkgState.zoom;
        if (sw < 26) continue; // zu klein für Beschriftung
        const p = pkgWorldToScreen(inf.sl.cx, inf.sl.cy);
        const col = inf.ar >= minAr ? "#4ade80" : (inf.ar >= minAr * 0.9 ? "#fbbf24" : "#f87171");
        const lines = [[`AR ${inf.ar.toFixed(2)}`, col]];
        if (inf.sl.type === "pin_in_paste") {
            const need = pipNeed(inf.sl.pin);
            if (need) {
                const pct = Math.round(inf.area / need * 100);
                lines.push([`${pct} % Vol.`, pct >= 95 ? "#4ade80" : (pct >= 80 ? "#fbbf24" : "#f87171")]);
            }
        }
        if (sw >= 95) lines.push([`${inf.area.toFixed(3)} mm²`, "#e2e8f0"], [`${inf.vol.toFixed(1)} nl`, "#cbd5e1"]);
        const fs = Math.max(9, Math.min(12, sw / 7));
        pkgCtx.font = `600 ${fs}px "JetBrains Mono", monospace`;
        const wbox = Math.min(sw, Math.max(...lines.map(l => pkgCtx.measureText(l[0]).width)) + 6);
        const hbox = lines.length * (fs + 2) + 2;
        pkgCtx.fillStyle = "rgba(0,0,0,0.6)";
        pkgCtx.fillRect(p.x - wbox / 2, p.y - hbox / 2, wbox, hbox);
        lines.forEach(([t, c], i) => {
            pkgCtx.fillStyle = c;
            pkgCtx.fillText(t, p.x, p.y - hbox / 2 + (fs + 2) * (i + 0.5) + 1, wbox);
        });
    }
    pkgCtx.restore();
}

// -------------------------------------------------------------
// Board-Mehrfachauswahl und direkte Bearbeitung von Öffnungen
// -------------------------------------------------------------
const selState = { ids: new Set(), box: null, addMode: false };

function setSelection(ids) {
    selState.ids = new Set(ids);
    updateSelBar();
    render();
}

function toggleSelection(id) {
    if (selState.ids.has(id)) selState.ids.delete(id); else selState.ids.add(id);
    updateSelBar();
    render();
}

function selectedApertures() {
    return state.apertures.filter(a => selState.ids.has(a.id));
}

// Pads, deren Mitte im Rahmen liegt, zur Auswahl hinzufügen
function selectInBox(a, b) {
    const [x0, x1] = [Math.min(a.x, b.x), Math.max(a.x, b.x)];
    const [y0, y1] = [Math.min(a.y, b.y), Math.max(a.y, b.y)];
    for (const ap of state.apertures) {
        if (ap.cx >= x0 && ap.cx <= x1 && ap.cy >= y0 && ap.cy <= y1) selState.ids.add(ap.id);
    }
    updateSelBar();
}

// "Gleiche" dazu wählen: gleiche Größe/Form, gleiches Package oder Bauteil
function selectSimilar(kind) {
    const sel = selectedApertures();
    if (!sel.length) return;
    const key = {
        size: a => `${a.shape}|${a.w.toFixed(3)}|${a.h.toFixed(3)}`,
        pkg: a => a.pkg_name || "",
        comp: a => a.comp_id || "",
    }[kind];
    const keys = new Set(sel.map(key).filter(k => k));
    for (const ap of state.apertures) if (keys.has(key(ap))) selState.ids.add(ap.id);
    updateSelBar();
    render();
    showToast(`${selState.ids.size} apertures selected`, 1500);
}

function updateSelBar() {
    const bar = document.getElementById("selBar");
    if (!bar) return;
    const n = selState.ids.size;
    bar.classList.toggle("hidden", n === 0);
    document.getElementById("selCount").textContent = n;
}

async function editSelection(op, extra = {}) {
    const ids = [...selState.ids];
    if (!ids.length) return;
    const res = await fetch(`/api/openings/${op}`, {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ ids, ...extra })
    });
    const d = await res.json();
    if (d.status !== "ok") { showToast(d.message || "Edit failed"); return; }
    if (op === "duplicate" && d.new_ids?.length) selState.ids = new Set(d.new_ids);
    await refreshApertureData();
    updateSelBar();
}

function selectionSizeArgs() {
    const w = parseFloat(document.getElementById("selW").value);
    const h = parseFloat(document.getElementById("selH").value);
    return { w: w > 0 ? w : null, h: h > 0 ? h : null };
}

// Neue Öffnung an der Klickstelle (Größe aus B × H, sonst 0,5 × 0,5 mm)
async function placeNewOpening(w) {
    setAddMode(false);
    const s = selectionSizeArgs();
    const res = await fetch("/api/opening/add", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ shape: "rectangle", cx: w.x, cy: w.y, w: s.w || 0.5, h: s.h || s.w || 0.5 })
    });
    const d = await res.json();
    if (d.status !== "ok") { showToast(d.message || "Aperture not created"); return; }
    selState.ids = new Set([d.id]);
    await refreshApertureData();
    updateSelBar();
    selectAperture(d.id);
}

function setAddMode(on) {
    selState.addMode = on;
    document.getElementById("btnAddOpening")?.classList.toggle("active", on);
    container.style.cursor = on ? "crosshair" : "";
}

// Tastatur im Board: liefert true, wenn die Taste verbraucht wurde
function handleSelectionKey(e) {
    const ctrl = e.ctrlKey || e.metaKey;
    if (ctrl && e.key.toLowerCase() === "a") { setSelection(state.apertures.map(a => a.id)); return true; }
    if (selState.addMode && e.key === "Escape") { setAddMode(false); return true; }
    if (!selState.ids.size) return false;
    const step = e.shiftKey ? 0.1 : 0.01;
    const moves = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] };
    if (moves[e.key]) { editSelection("move", { dx: moves[e.key][0], dy: moves[e.key][1] }); return true; }
    if (ctrl && e.key.toLowerCase() === "d") { editSelection("duplicate", { dx: 0.5, dy: 0 }); return true; }
    if (ctrl) return false;
    if ((e.key === "Delete" || e.key === "Backspace") && !measureState.measurements.length) { editSelection("delete"); return true; }
    if ((e.key === "r" || e.key === "R") && !measureState.active) { editSelection("rotate", { deg: e.shiftKey ? -90 : 90 }); return true; }
    if (e.key === "Escape" && !measureState.active && !measureState.measurements.length) { setSelection([]); return true; }
    return false;
}

function renderMultiSelection() {
    // Nach Undo/Neuladen: nicht mehr vorhandene Öffnungen aus der Auswahl nehmen
    for (const id of selState.ids) if (!state.apertures[id] || state.apertures[id].id !== id) selState.ids.delete(id);
    if (selState.ids.size) {
        ctx.save();
        ctx.strokeStyle = "#facc15";
        ctx.lineWidth = 1.5;
        for (const ap of selectedApertures()) {
            const sp = worldToScreen(ap.cx - ap.w / 2, ap.cy + ap.h / 2);
            ctx.strokeRect(sp.x - 2, sp.y - 2, Math.max(2, ap.w * state.zoom) + 4, Math.max(2, ap.h * state.zoom) + 4);
        }
        ctx.restore();
    }
    if (selState.box) {
        const a = worldToScreen(selState.box.start.x, selState.box.start.y);
        const b = worldToScreen(selState.box.cur.x, selState.box.cur.y);
        ctx.save();
        ctx.fillStyle = "rgba(250, 204, 21, 0.08)";
        ctx.strokeStyle = "#facc15";
        ctx.setLineDash([4, 3]);
        ctx.fillRect(Math.min(a.x, b.x), Math.min(a.y, b.y), Math.abs(b.x - a.x), Math.abs(b.y - a.y));
        ctx.strokeRect(Math.min(a.x, b.x), Math.min(a.y, b.y), Math.abs(b.x - a.x), Math.abs(b.y - a.y));
        ctx.restore();
    }
}

// Verlauf: Liste aller Schritte, Klick springt dorthin (mehrere Schritte zurück/vor)
function renderHistoryMenu() {
    const menu = document.getElementById("historyMenuList");
    const h = projectState.history || {};
    const undo = h.undo_labels || [], redo = h.redo_labels || [];
    menu.innerHTML = "";
    const add = (label, cls, fn) => {
        const b = document.createElement("button");
        b.textContent = label;
        if (cls) b.className = cls;
        if (fn) b.addEventListener("click", fn);
        menu.appendChild(b);
    };
    // Wiederholbare Schritte oben (der späteste ganz oben), dann der aktuelle Stand, dann die Rückgängig-Schritte
    [...redo].reverse().forEach((l, i) => add(`↷ ${l}`, "redo", () => jumpHistory("redo", redo.length - i)));
    add("● Current State", "current");
    undo.forEach((l, i) => add(`↶ ${l}`, "", () => jumpHistory("undo", i + 1)));
}

async function jumpHistory(dir, steps) {
    document.getElementById("historyMenuList").classList.add("hidden");
    const d = await (await fetch(`/api/${dir}`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ steps }) })).json();
    if (d.status === "ok") { showToast(`${dir === "undo" ? "↶" : "↷"} ${steps} step(s): ${d.label}`, 2000); await reloadAllViews(); }
    refreshHistory();
}

window.addEventListener("DOMContentLoaded", () => {
    document.querySelectorAll("#selBar [data-similar]").forEach(b => b.addEventListener("click", () => selectSimilar(b.dataset.similar)));
    document.querySelectorAll("#selBar [data-op]").forEach(b => b.addEventListener("click", () => {
        const op = b.dataset.op;
        if (op === "reduce") editSelection("reduce", { percent: parseFloat(document.getElementById("selReducePct").value) || 10 });
        else if (op === "resize") {
            const s = selectionSizeArgs();
            if (!s.w && !s.h) { showToast("Enter width and/or height"); return; }
            editSelection("resize", s);
        }
        else if (op === "rotate") editSelection("rotate", { deg: 90 });
        else if (op === "duplicate") editSelection("duplicate", { dx: 0.5, dy: 0 });
        else editSelection(op);
    }));
    document.getElementById("btnSelClear")?.addEventListener("click", () => setSelection([]));
    // Zone um die Auswahl: Umriss der Öffnungen plus Abstand zur Stufenkante, dann der übliche Zonen-Dialog
    document.getElementById("btnSelZone")?.addEventListener("click", () => {
        const aps = selectedApertures();
        if (!aps.length) return;
        const [mx, my] = state.zoneMargins || [1, 1];
        openZoneModal(Math.min(...aps.map(a => a.cx - a.w / 2)) - mx, Math.min(...aps.map(a => a.cy - a.h / 2)) - my,
                      Math.max(...aps.map(a => a.cx + a.w / 2)) + mx, Math.max(...aps.map(a => a.cy + a.h / 2)) + my);
    });
    document.getElementById("btnAddOpening")?.addEventListener("click", () => setAddMode(!selState.addMode));
    const menu = document.getElementById("historyMenuList");
    document.getElementById("btnHistory")?.addEventListener("click", async (e) => {
        e.stopPropagation();
        await refreshHistory();
        renderHistoryMenu();
        menu.classList.toggle("hidden");
    });
    document.addEventListener("click", (e) => { if (!e.target.closest("#btnHistory")) menu?.classList.add("hidden"); });
});

// -------------------------------------------------------------
// Stencil-Analyse: Tabelle aller Öffnungen (Filter, Sortierung, Sprung, Auswahl, CSV)
// -------------------------------------------------------------
const anState = { rows: [], sortKey: "status", sortDir: 1 };
const AN_STATUS_ORDER = { CRIT: 0, WARN: 1, OK: 2 };

function analysisLimits() {
    return { cov_min: parseFloat(document.getElementById("anCovMin").value) || 0, cov_max: parseFloat(document.getElementById("anCovMax").value) || 1e9 };
}

async function loadAnalysis() {
    const res = await fetch("/api/analysis", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(analysisLimits()) });
    const d = await res.json();
    anState.rows = d.rows || [];
    renderAnalysis();
}

function filteredAnalysisRows() {
    const mode = document.getElementById("anStatus").value;
    const q = document.getElementById("anSearch").value.trim().toLowerCase();
    let rows = anState.rows.filter(r => mode === "all" || (mode === "CRIT" ? r.status === "CRIT" : r.status !== "OK"));
    if (q) rows = rows.filter(r => `${r.comp} ${r.pkg} ${r.issues.join(" ")}`.toLowerCase().includes(q));
    const k = anState.sortKey, dir = anState.sortDir;
    const val = r => k === "status" ? AN_STATUS_ORDER[r.status] : k === "issues" ? r.issues.join(", ") : r[k];
    return rows.sort((a, b) => {
        const x = val(a), y = val(b);
        if (x == null) return 1;
        if (y == null) return -1;
        return (typeof x === "number" ? x - y : String(x).localeCompare(String(y), "de", { numeric: true })) * dir;
    });
}

function renderAnalysis() {
    const rows = filteredAnalysisRows();
    const counts = anState.rows.reduce((c, r) => (c[r.status]++, c), { OK: 0, WARN: 0, CRIT: 0 });
    document.getElementById("anSummary").textContent =
        `${anState.rows.length} apertures · ${counts.CRIT} critical · ${counts.WARN} warning · ${rows.length} shown`;
    const f = (v, n) => v == null ? "–" : v.toFixed(n);
    // shortcut: höchstens 2000 Zeilen im DOM, Filter einschränken; virtuelle Liste, falls das nicht reicht
    document.getElementById("anBody").innerHTML = rows.slice(0, 2000).map(r => `
        <tr data-id="${r.id}" class="an-${r.status.toLowerCase()}">
            <td><span class="badge ${r.status === "CRIT" ? "crit" : r.status === "WARN" ? "warn" : "ok"}">${r.status === "CRIT" ? "Critical" : r.status === "WARN" ? "Warning" : "OK"}</span></td>
            <td>${escapeHtml(r.comp || "–")}</td><td>${r.pin}</td><td>${escapeHtml(r.pkg || "–")}</td>
            <td>${f(r.ar, 2)}</td><td>${f(r.aspect, 2)}</td><td>${f(r.min_w, 3)}</td>
            <td>${f(r.area, 4)}</td><td>${f(r.volume, 4)}</td><td>${r.coverage == null ? "–" : r.coverage.toFixed(0) + " %"}</td>
            <td>${escapeHtml(r.issues.join(", "))}${r.manual ? ' <span class="an-manual" title="Edited manually">✎</span>' : ""}</td>
        </tr>`).join("");
    document.querySelectorAll("#analysisModal th[data-k]").forEach(th => th.classList.toggle("sorted", th.dataset.k === anState.sortKey));
}

function jumpToAperture(id) {
    const a = state.apertures[id];
    if (!a) return;
    fitToWorldBox(a.cx - 2, a.cy - 2, a.cx + 2, a.cy + 2);
    setSelection([id]);
    selectAperture(id);
}

window.addEventListener("DOMContentLoaded", () => {
    const modal = document.getElementById("analysisModal");
    if (!modal) return;
    const close = () => modal.classList.add("hidden");
    document.getElementById("btnAnalysis")?.addEventListener("click", async () => { modal.classList.remove("hidden"); await loadAnalysis(); });
    document.getElementById("btnCloseAnalysis").addEventListener("click", close);
    document.getElementById("btnCloseAnalysis2").addEventListener("click", close);
    document.getElementById("anStatus").addEventListener("change", renderAnalysis);
    document.getElementById("anSearch").addEventListener("input", renderAnalysis);
    ["anCovMin", "anCovMax"].forEach(i => document.getElementById(i).addEventListener("change", loadAnalysis));
    document.querySelectorAll("#analysisModal th[data-k]").forEach(th => th.addEventListener("click", () => {
        anState.sortDir = anState.sortKey === th.dataset.k ? -anState.sortDir : 1;
        anState.sortKey = th.dataset.k;
        renderAnalysis();
    }));
    document.getElementById("anBody").addEventListener("click", (e) => {
        const tr = e.target.closest("tr[data-id]");
        if (!tr) return;
        close();
        jumpToAperture(+tr.dataset.id);
    });
    document.getElementById("btnAnSelect").addEventListener("click", () => {
        const ids = filteredAnalysisRows().map(r => r.id);
        close();
        setSelection(ids);
        showToast(`${ids.length} apertures selected – now edit together`, 2500);
    });
    document.getElementById("btnAnCsv").addEventListener("click", async () => {
        const d = await (await fetch("/api/analysis/csv", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(analysisLimits()) })).json();
        showToast(d.status === "ok" ? `CSV saved: ${d.path}` : `CSV export failed: ${d.message}`, 5000);
    });
});
