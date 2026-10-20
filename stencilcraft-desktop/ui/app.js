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
    ruleConfigCard: "--rule-card-h"
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
        const horiz = sp.classList.contains("splitter-h");
        const side = sp.dataset.side;
        sp.addEventListener("mousedown", (e) => {
            e.preventDefault();
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
                root.style.setProperty(PANEL_VARS[target.id], `${Math.round(size)}px`);
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
        state.pads = data.pads || [];
        state.components = data.components || [];
        state.zones = data.zones;
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

async function updateStats() {
    const res = await fetch("/api/stats");
    const data = await res.json();
    state.stats = data.stats;

    document.getElementById("statTotal").innerText = state.stats.total_apertures.toLocaleString();
    document.getElementById("statOk").innerText = state.stats.ok_count.toLocaleString();
    document.getElementById("statWarn").innerText = state.stats.warn_count.toLocaleString();
    document.getElementById("statCrit").innerText = state.stats.crit_count.toLocaleString();
    document.getElementById("zonesCount").innerText = state.stats.zones_count;
    document.getElementById("lblMinAr").innerText = state.stats.ipc_min_ar.toFixed(2);

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
function pkgWorldToScreen(x, y) {
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
    return { x: vx * c + vy * s, y: vx * s - vy * c };
}

// Fangpunkte eines Elements: Zentrum, Ecken, Kantenmitten bzw. Polygon-Eckpunkte
function snapCandidates(cx, cy, w, h, pts, round) {
    const out = [{ x: cx, y: cy, kind: "Mitte" }];
    if (pts && pts.length >= 3 && pts.length <= 64) {
        for (const p of pts) out.push({ x: p[0], y: p[1], kind: "Ecke" });
        for (let i = 0; i < pts.length; i++) {
            const a = pts[i], b = pts[(i + 1) % pts.length];
            out.push({ x: (a[0] + b[0]) / 2, y: (a[1] + b[1]) / 2, kind: "Kante" });
        }
    } else if (w && h) {
        const hw = w / 2, hh = h / 2;
        out.push({ x: cx - hw, y: cy, kind: "Kante" }, { x: cx + hw, y: cy, kind: "Kante" },
                 { x: cx, y: cy - hh, kind: "Kante" }, { x: cx, y: cy + hh, kind: "Kante" });
        if (!round) {
            out.push({ x: cx - hw, y: cy - hh, kind: "Ecke" }, { x: cx + hw, y: cy - hh, kind: "Ecke" },
                     { x: cx + hw, y: cy + hh, kind: "Ecke" }, { x: cx - hw, y: cy + hh, kind: "Ecke" });
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
    return best;
}

// Fangen im Board: Pad-/Slice-Ecken, Kantenmitten, Zentren und Bauteilzentren (8 px Fangradius).
function findSnapPointBoard(wx, wy) {
    const snapDistMm = 8 / state.zoom;
    const items = [];
    for (const ap of state.apertures || []) {
        const label = ap.comp_id ? `${ap.comp_id}` : "Pad";
        if (ap.slices && ap.slices.length > 0) {
            for (const s of ap.slices) items.push({ cx: s.cx, cy: s.cy, w: s.w, h: s.h, pts: s.pts, round: s.shape === "circle", label });
        }
        items.push({ cx: ap.cx, cy: ap.cy, w: ap.w, h: ap.h, pts: ap.pts, round: ap.shape === "circle", label });
    }
    for (const c of state.components || []) {
        items.push({ cx: c.cx, cy: c.cy, w: 0, h: 0, label: c.refdes || "Bauteil" });
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

    targetCtx.fillStyle = "rgba(15, 23, 42, 0.90)";
    targetCtx.strokeStyle = color;
    targetCtx.lineWidth = 1;
    targetCtx.beginPath();
    targetCtx.roundRect(midX - padW / 2, midY - padH / 2, padW, padH, 5);
    targetCtx.fill();
    targetCtx.stroke();

    targetCtx.fillStyle = "#ffffff";
    targetCtx.font = "bold 11px Inter, sans-serif";
    targetCtx.textAlign = "center";
    targetCtx.textBaseline = "middle";
    targetCtx.fillText(line1, midX, midY - 6);

    targetCtx.fillStyle = "#94a3b8";
    targetCtx.font = "9px Inter, sans-serif";
    targetCtx.fillText(line2, midX, midY + 7);

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
        showToast("📏 Messwerkzeug aktiv: 1. Punkt klicken, 2. Zielpunkt klicken");
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
    showToast("Messungen gelöscht");
    render();
    renderPkgPreview();
}

function fitToScreen() {
    const rect = container.getBoundingClientRect();
    let [minX, minY, maxX, maxY] = state.bounds || [0, 0, 0, 0];
    if (minX === 0 && maxX === 0 && state.apertures && state.apertures.length > 0) {
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
        ctx.fillText("⚡ SMT StencilCraft bereit", rect.width / 2, rect.height / 2 - 15);

        ctx.fillStyle = "rgba(100, 116, 139, 0.9)";
        ctx.font = "14px Inter, sans-serif";
        ctx.fillText("Klicken Sie oben auf '📂 ODB++ / Gerber öffnen' oder ziehen Sie eine Datei / ZIP hierher.", rect.width / 2, rect.height / 2 + 18);
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

    // 9. Selected Aperture Highlight
    if (state.selectedAperture) {
        renderSelectionHighlight();
    }

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

function renderComponents() {
    const compLayer = state.layers ? state.layers.find(l => l.type === 'component') : null;
    if (compLayer && !compLayer.visible) return;
    if (!state.showComponents) return;

    ctx.strokeStyle = "rgba(250, 204, 21, 0.75)";
    ctx.fillStyle = "#facc15";
    ctx.lineWidth = 1;
    ctx.font = "bold 9px Inter, sans-serif";

    for (const c of state.components) {
        const center = worldToScreen(c.cx, c.cy);
        const sw = Math.max(2, c.width * state.zoom);
        const sh = Math.max(2, c.height * state.zoom);

        ctx.save();
        ctx.translate(center.x, center.y);
        if (c.rot) {
            ctx.rotate(-c.rot * Math.PI / 180.0);
        }
        ctx.strokeRect(-sw / 2, -sh / 2, sw, sh);

        if (state.zoom > 2.5 && c.refdes) {
            ctx.fillText(c.refdes, -sw / 2 + 2, -sh / 2 - 2);
        }
        ctx.restore();
    }
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
    }
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

function renderApertures() {
    const pasteLayers = state.layers ? state.layers.filter(l => l.type === 'paste') : [];
    if (pasteLayers.length > 0) {
        const anyPasteVisible = pasteLayers.some(l => l.visible);
        if (!anyPasteVisible) return;
    } else {
        if (!state.showPaste) return;
    }

    const isHeatmap = state.colorMode === "ipc";
    const isLayers = state.colorMode === "layers";
    const activePasteLayer = pasteLayers.find(l => l.visible);
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
            fillColor = pasteColor; // Pink Paste wie im Kunden-Screenshot
        } else if (state.colorMode === "thickness") {
            fillColor = ap.thick < 0.120 ? "#c084fc" : (ap.thick > 0.120 ? "#f43f5e" : "#38bdf8");
        }

        ctx.fillStyle = fillColor;

        // Wenn Slices oder Länglich&Schmal aktiv
        if (ap.slices && ap.slices.length > 0) {
            for (const s of ap.slices) {
                if (s.pts && s.pts.length > 0) {
                    ctx.beginPath();
                    for (let i = 0; i < s.pts.length; i++) {
                        const sp = worldToScreen(s.pts[i][0], s.pts[i][1]);
                        if (i === 0) ctx.moveTo(sp.x, sp.y);
                        else ctx.lineTo(sp.x, sp.y);
                    }
                    ctx.closePath();
                    ctx.fill();
                } else {
                    const sp = worldToScreen(s.cx - s.w/2, s.cy + s.h/2);
                    const sw = Math.max(1, s.w * state.zoom);
                    const sh = Math.max(1, s.h * state.zoom);
                    ctx.fillRect(sp.x, sp.y, sw, sh);
                }
            }
            continue;
        }

        if (ap.shape === "circle") {
            const sp = worldToScreen(ap.cx, ap.cy);
            const sr = Math.max(1, (ap.w / 2) * state.zoom);
            ctx.beginPath();
            ctx.arc(sp.x, sp.y, sr, 0, Math.PI * 2);
            ctx.fill();
        } else if (ap.shape === "rectangle" || !ap.pts) {
            const sp = worldToScreen(ap.cx - ap.w/2, ap.cy + ap.h/2);
            const sw = Math.max(1, ap.w * state.zoom);
            const sh = Math.max(1, ap.h * state.zoom);
            ctx.fillRect(sp.x, sp.y, sw, sh);
        } else if (ap.shape === "polygon" && ap.pts) {
            ctx.beginPath();
            for (let i = 0; i < ap.pts.length; i++) {
                const sp = worldToScreen(ap.pts[i][0], ap.pts[i][1]);
                if (i === 0) ctx.moveTo(sp.x, sp.y);
                else ctx.lineTo(sp.x, sp.y);
            }
            ctx.closePath();
            ctx.fill();
        }
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
    el.innerText = `Zoom: ${zPct}% | Aperturen: ${state.apertures.length}`;
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

function updateInspectorUI(ap) {
    document.getElementById("noSelectionMsg").classList.add("hidden");
    document.getElementById("selectionDetails").classList.remove("hidden");

    document.getElementById("padId").innerText = ap.id;
    document.getElementById("padDim").innerText = `${ap.width.toFixed(2)} x ${ap.height.toFixed(2)} mm`;
    document.getElementById("padPos").innerText = `X: ${ap.cx.toFixed(2)} | Y: ${ap.cy.toFixed(2)}`;

    const thickUm = Math.round(ap.ipc.effective_thickness * 1000);
    const zoneText = ap.ipc.zone_id ? ` (Zone)` : ` (Basis)`;
    document.getElementById("padThick").innerText = `${thickUm} µm${zoneText}`;
    document.getElementById("padIdentical").innerText = `${ap.identical_count} auf Platine`;
    document.querySelectorAll(".lblIdenticalCount").forEach(el => el.innerText = ap.identical_count);

    const badge = document.getElementById("padStatusBadge");
    badge.className = `badge ${ap.ipc.status.toLowerCase()}`;
    badge.innerText = ap.ipc.status;

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
            if (msgEl) msgEl.innerText = ap.ipc.grain_msg || "Mindest-Aperturbreite für Pastentyp unterschritten!";
        } else {
            grAlert.classList.add("hidden");
        }
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
                    <span>Fläche:</span>
                    <span>${areaStr}</span>
                </div>
                <div class="calc-excel-row">
                    <span>Wandfläche:</span>
                    <span>${wallStr}</span>
                </div>
                <div class="calc-excel-row highlight-ar ${arClass}">
                    <span>Area Ratio (AR):</span>
                    <span>${r.area_ratio.toFixed(2)}</span>
                </div>
                <div class="calc-excel-row">
                    <span>Volumen:</span>
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

function renderLayerTable() {
    const tbody = document.getElementById("layerTableBody");
    if (!tbody) return;
    tbody.innerHTML = "";

    const countBadge = document.getElementById("layerCountBadge");
    if (countBadge) countBadge.innerText = state.layers.length;

    for (const layer of state.layers) {
        const tr = document.createElement("tr");
        if (!layer.visible) tr.classList.add("row-hidden");

        const isVisible = layer.visible !== false;
        const color = layer.color || "#c084fc";
        const typeBadge = layer.type ? layer.type.toLowerCase() : "other";

        tr.innerHTML = `
            <td class="col-chk">
                <input type="checkbox" class="layer-chk-input" data-layer-id="${layer.id}" ${isVisible ? 'checked' : ''} title="Lage ein-/ausblenden (Haken)">
            </td>
            <td class="col-color">
                <input type="color" class="layer-color-picker" data-layer-id="${layer.id}" value="${color}" title="Farbe ändern">
            </td>
            <td class="layer-name-cell" title="${layer.name} (${layer.filename || ''})">
                <div style="font-weight:600;">${escapeHtml(layer.name)}</div>
                <div class="layer-subname">${escapeHtml(layer.filename || '')}</div>
            </td>
            <td class="col-type">
                <span class="badge-layer ${typeBadge}">${layer.type || 'LAYER'}</span>
            </td>
            <td class="col-count layer-count-cell">
                ${(layer.count || 0).toLocaleString()}
            </td>
            <td class="col-del">
                <button class="layer-del-btn" data-layer-id="${layer.id}" title="Lage entfernen">✕</button>
            </td>
        `;

        // Checkbox "Haken" Toggle Event -> Sofortiges 0ms Canvas update!
        const chk = tr.querySelector(".layer-chk-input");
        chk.addEventListener("change", (e) => {
            const checked = e.target.checked;
            layer.visible = checked;
            if (state.layerPrimitives[layer.id]) {
                state.layerPrimitives[layer.id].visible = checked;
            }
            if (checked) {
                tr.classList.remove("row-hidden");
            } else {
                tr.classList.add("row-hidden");
            }
            render();

            // Im Hintergrund synchronisieren
            fetch(`/api/layer/${layer.id}/toggle`, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ visible: checked })
            });
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
            if (confirm(`Möchten Sie die Lage "${layer.name}" wirklich entfernen?`)) {
                await fetch(`/api/layer/${layer.id}`, { method: "DELETE" });
                delete state.layerPrimitives[layer.id];
                state.layers = state.layers.filter(l => l.id !== layer.id);
                renderLayerTable();
                render();
            }
        });

        tbody.appendChild(tr);
    }
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
    toast.innerHTML = `<span>⚡</span> <span>${msg}</span>`;
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
    return guardUnsaved("einen neuen Datensatz laden");
}

// Windows Dialog Triggers
async function triggerWindowsFileDialog(clearPrevious = true) {
    if (clearPrevious) {
        if (!await promptCloseBeforeLoad()) return;
    }
    const toast = showToast("📂 Windows-Dateiauswahl geöffnet... Bitte wählen Sie Dateien im Explorer.", 0);
    try {
        const res = await fetch("/api/open_windows_file_dialog", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ side: "top", clear_previous: clearPrevious })
        });
        toast.remove();
        const data = await res.json();
        if (data.status === "ok") {
            showToast(`✅ ${data.count} Lage(n) erfolgreich importiert!`);
            await loadLayers();
            await refreshApertureData();
            fitToScreen();
            document.getElementById("zipModal")?.classList.add("hidden");
        } else if (data.status === "cancelled") {
            showToast("ℹ️ Dateiauswahl abgebrochen");
        } else {
            showToast(`❌ Fehler: ${data.message || 'Unbekannt'}`);
        }
    } catch (e) {
        toast.remove();
        showToast("❌ Fehler beim Aufruf der Windows-Dateiauswahl");
    }
}

async function triggerWindowsFolderDialog() {
    if (!await promptCloseBeforeLoad()) return;
    const toast = showToast("📂 Windows-Ordnerauswahl geöffnet... Bitte wählen Sie einen Ordner im Explorer.", 0);
    try {
        const res = await fetch("/api/open_windows_folder_dialog", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ side: "top" })
        });
        toast.remove();
        const data = await res.json();
        if (data.status === "ok") {
            showToast(`✅ Ordner importiert (${data.count} Lagen)!`);
            await loadLayers();
            await refreshApertureData();
            fitToScreen();
            document.getElementById("zipModal")?.classList.add("hidden");
        } else if (data.status === "cancelled") {
            showToast("ℹ️ Ordnerauswahl abgebrochen");
        } else {
            showToast(`❌ Fehler: ${data.message || 'Unbekannt'}`);
        }
    } catch (e) {
        toast.remove();
        showToast("❌ Fehler beim Aufruf der Windows-Ordnerauswahl");
    }
}

async function uploadFilesFromBrowser(files) {
    if (!files || files.length === 0) return;
    if (!await promptCloseBeforeLoad()) return;
    const toast = showToast(`⬆️ Lade ${files.length} Datei(en) hoch...`, 0);
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
            showToast(`✅ ${data.count} Lage(n) importiert!`);
            await loadLayers();
            await refreshApertureData();
            fitToScreen();
        } else {
            showToast(`❌ Fehler: ${data.message}`);
        }
    } catch (e) {
        toast.remove();
        showToast("❌ Upload fehlgeschlagen");
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
            showToast(`📦 Package "${targetPkg}" im Stencil Generator geöffnet (${targetRot}°)`);
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
        if (state.isDragging) {
            const dist = Math.hypot(e.clientX - state.dragStartX, e.clientY - state.dragStartY);
            state.isDragging = false;
            if (dist < 4) {
                const rect = container.getBoundingClientRect();
                handleCanvasClick(e.clientX - rect.left, e.clientY - rect.top);
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

    function handleCanvasClick(sx, sy) {
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

        if (bestItem) {
            selectAperture(bestItem.id);
        }
    }

    // Header Controls
    document.getElementById("datasetSelect")?.addEventListener("change", (e) => {
        loadDataset(e.target.value);
    });

    document.getElementById("btnOpenDataset")?.addEventListener("click", () => triggerWindowsFileDialog(true));
    document.getElementById("btnAddMoreLayers")?.addEventListener("click", () => triggerWindowsFileDialog(false));

    document.getElementById("ipcModeSelect")?.addEventListener("change", async (e) => {
        const mode = e.target.value;
        await fetch("/api/set_ipc_mode", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ mode })
        });
        await refreshApertureData();
    });

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
        showToast("🔄 Lagen aktualisiert");
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
            showToast(`Pastentyp gewechselt auf ${data.paste_type.toUpperCase()} (5-Ball-Regel aktiv)`);
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
                document.getElementById("pipTargetInfo").innerText = `Ziel: Apertur #${state.selectedAperture.id} (${state.selectedAperture.width.toFixed(2)} × ${state.selectedAperture.height.toFixed(2)} mm)`;
            } else {
                document.getElementById("pipTargetInfo").innerText = `Hinweis: Kein Pad gewählt (Vorschau-Modus)`;
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
        if (lbl) lbl.innerText = isRect ? "Pin-Breite (mm):" : "Pin-Ø (mm):";
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
            alert("Bitte wählen Sie zuerst eine Apertur / ein Pad auf der Platine aus!");
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
            showToast("✅ ZIP-Datensatz geladen!");
        } else {
            alert("Fehler beim Laden der ZIP: " + data.message);
        }
    });

    // EXPORT
    document.getElementById("btnExport").addEventListener("click", openExportDialog);
    setupExportDialog();

    // Datensatz schließen (mit Abfrage bei ungespeicherten Änderungen)
    document.getElementById("btnCloseDataset")?.addEventListener("click", async () => {
        if (!(await guardUnsaved("den Datensatz schließen"))) return;
        await fetch("/api/project/discard_recovery", { method: "POST" });
        await closeCurrentDataset();
    });
}

async function triggerExport() {
    try {
        const res = await fetch("/api/export", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
        const data = await res.json();
        showToast("💾 Gerber-Dateien exportiert!");
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
        showToast("✕ Datensatz geschlossen");
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
        list.innerHTML = `<p class="empty-state">Keine Stufenzonen definiert.</p>`;
        return;
    }

    for (const z of state.zones) {
        const card = document.createElement("div");
        card.className = "zone-card";
        const thickUm = Math.round(z.thickness * 1000);
        card.innerHTML = `
            <div class="zone-card-info">
                <h4>${z.name}</h4>
                <div class="zone-card-meta">${z.width} x ${z.height} mm | Dicke: <strong>${thickUm} µm</strong></div>
            </div>
            <div style="display: flex; gap: 6px;">
                <button class="btn btn-sm btn-secondary btn-edit-thick" data-id="${z.id}" data-thick="${thickUm}">✏️ Dicke</button>
                <button class="btn btn-sm btn-danger btn-del-zone" data-id="${z.id}">Löschen</button>
            </div>
        `;

        card.querySelector(".btn-edit-thick").addEventListener("click", async (e) => {
            e.stopPropagation();
            const cur = card.querySelector(".btn-edit-thick").dataset.thick;
            const neu = prompt(`Neue Zonendicke für "${z.name}" in µm eingeben:`, cur);
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
            await refreshApertureData();
        });
        list.appendChild(card);
    }
}

function renderCriticalList() {
    const list = document.getElementById("criticalList");
    list.innerHTML = "";

    if (!state.stats || !state.stats.top_critical_packages || state.stats.top_critical_packages.length === 0) {
        list.innerHTML = `<p class="empty-state">Keine IPC-Verletzungen gefunden! Alle Aperturen lösen sicher aus. 🎉</p>`;
        return;
    }

    for (const p of state.stats.top_critical_packages) {
        const card = document.createElement("div");
        card.className = "critical-card";
        card.innerHTML = `
            <div class="crit-header">
                <span>Package: ${p.group} mm</span>
                <span class="badge crit">${p.crit_count} kritisch</span>
            </div>
            <div class="crit-meta">Insgesamt ${p.total} Pads auf dem Panel. Klicken zum Anspringen.</div>
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
        const factor = e.deltaY < 0 ? 1.15 : 0.85;
        pkgState.zoom = Math.max(10, Math.min(400, pkgState.zoom * factor));
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

        if (clickedPad) {
            selectPkgPin(clickedPad.pin, clickedPad);
        } else {
            pkgState.isDragging = true;
            pkgState.dragStartX = e.clientX - pkgState.panX;
            pkgState.dragStartY = e.clientY - pkgState.panY;
        }
    });

    window.addEventListener("mousemove", (e) => {
        if (pkgState.isDragging) {
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
    });

    // Rechtsklick im Stencil Generator -> Messungen sofort löschen
    pkgContainer.addEventListener("contextmenu", (e) => {
        if ((measureState.measurements && measureState.measurements.length > 0) || measureState.active) {
            e.preventDefault();
            clearMeasurements();
            if (measureState.active) toggleMeasureTool(false);
        }
    });

    // Doppelklick auf Pad im Package-Modus -> Pin auswählen und Eingabe fokussieren
    pkgContainer.addEventListener("dblclick", (e) => {
        if (e.target !== pkgCanvas || measureState.active) return;
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
    } catch (e) {
        console.error("Fehler beim Laden der Bibliotheken:", e);
    }
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
    { key: "Library", label: "Bibliothek (nicht auf Platine)" }
];

function renderPkgTree() {
    const list = document.getElementById("pkgTreeList");
    list.innerHTML = "";

    const query = pkgState.searchQuery.toLowerCase();
    const filtered = pkgState.packages.filter(p => {
        if (pkgState.activeCategory && (p.group || "SMD") !== pkgState.activeCategory) return false;
        if (query && !p.name.toLowerCase().includes(query)) return false;
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
        header.innerHTML = `<span class="caret">${collapsed ? "▸" : "▾"}</span><span class="grp-label" title="Gruppenregel bearbeiten">${g.label}</span><span class="pkg-group-count">${members.length}</span>`;
        header.querySelector(".caret").addEventListener("click", (e) => {
            e.stopPropagation();
            pkgState.collapsedGroups[g.key] = !pkgState.collapsedGroups[g.key];
            renderPkgTree();
        });
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
        item.className = `pkg-tree-item ${p.name === pkgState.activePkgName && pkgState.ruleTarget?.kind === "package" ? 'active' : ''} ${inherits ? 'inherits' : ''}`;
        item.title = inherits ? "Regel von der Gruppe geerbt" : `Eigene Regel: ${p.rule?.rule_type}`;
        
        let icon = "📦";
        if (p.category === "SMD Passives") icon = "▫️";
        else if (p.category === "ICs & Transistors") icon = "🔲";
        else if (p.category === "BGA & Arrays") icon = "▦";

        item.innerHTML = `
            <div class="pkg-item-main">
                <span class="pkg-item-icon">${icon}</span>
                <span class="pkg-item-name" title="${p.name}">${p.name}</span>${inherits ? "" : '<span class="own-rule-dot">●</span>'}
            </div>
            <div class="pkg-item-badges">
                ${p.pitch > 0 ? `<span class="pkg-badge-pitch">${p.pitch}mm</span>` : ''}
                ${p.component_count > 0 ? `<span class="pkg-badge-count" title="${p.component_count}x auf Platine">${p.component_count}x</span>` : ''}
            </div>
        `;

        item.addEventListener("click", () => {
            selectPackage(p.name);
        });

        list.appendChild(item);

        // Aktives Package: Pins als Unterknoten (wie PCB Investigator "1(0)", "2(1)")
        const data = pkgState.activePackageData;
        if (p.name === pkgState.activePkgName && data && data.name === p.name && data.pads?.length) {
            const pins = document.createElement("div");
            pins.className = "pkg-pin-list";
            const ovs = data.pad_overrides || {};
            data.pads.forEach((pad, idx) => {
                const pi = document.createElement("div");
                const hasOv = ovs[pad.pin] || ovs[String(pad.pin)];
                pi.className = `pkg-pin-item ${pkgState.selectedPin === pad.pin ? "active" : ""}`;
                pi.innerHTML = `<span>${pad.pin}(${idx})</span><span>${pad.w}×${pad.h}${pad.drill ? ` ⌀${pad.drill}` : ""}</span>${hasOv ? '<span class="pin-ov">✏️</span>' : ""}`;
                pi.addEventListener("click", (e) => {
                    e.stopPropagation();
                    selectPkgPin(pad.pin, pad);
                    renderPkgTree();
                });
                pins.appendChild(pi);
            });
            list.appendChild(pins);
        }
    }
}

async function selectPackage(pkgName) {
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
    select.innerHTML = `<option value="all">Alle Pins (Ganzes Package)</option>`;
    if (!pads || pads.length === 0) return;
    for (const p of pads) {
        const opt = document.createElement("option");
        opt.value = p.pin;
        const hasOv = overrides && (overrides[p.pin] || overrides[String(p.pin)]);
        opt.textContent = `Pin ${p.pin} (${p.w}x${p.h}mm)${hasOv ? " ✏️ (angepasst)" : ""}`;
        select.appendChild(opt);
    }
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
    const spanX = Math.max(2.0, (maxX - minX) * 1.6);
    const spanY = Math.max(2.0, (maxY - minY) * 1.6);

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
    updateRuleFieldVisibility();
}

// Beschreibung der Regeln (wie im PCB Investigator unter der Parameterliste)
const RULE_DESCRIPTIONS = {
    package_group: "Wählt die Öffnung je Pad automatisch: BGA-Pads 1:1, große Flächen als Array-Felder (Mindestfeldgröße, Feldabstand, Flächennutzung), 2-Pin-Chips als HouseBuilder (kleine als Inverted House), sonst Rechteck mit Undersize und Eckradius.",
    simple_rect: "Create one surface with fix size and offset away from the center. Optional mit Stegen (Bridge Count) und fester Größe.",
    house_builder: "Hausform: Spitze zum Bauteilkörper, verhindert Tombstoning und Lotperlen bei Chip-Bauteilen.",
    inverted_house_builder: "Umgekehrte Hausform: Kerbe zur Bauteilaußenseite.",
    elongated_pitch: "Länglich & Schmal: verschmälert quer zum Pitch (Pastenabstand = Faktor × Öffnungsbreite), verlängert im Verhältnis zur Verschmälerung und versetzt die Öffnung nach außen.",
    array_slicing: "Teilt die Fläche in Felder mit Stegen (Bridge Count / Width), optional 45° gedreht.",
    donut: "Ringöffnung um die Bohrung (THT / Pin-in-Paste), segmentiert mit Haltestegen (Bridge Width).",
    take_existing: "Übernimmt die Pastenöffnungen unverändert aus dem Kundendatensatz.",
    inherit: "Inherit: Das Package übernimmt die Regel seiner Gruppe (SMD/THT/Other/Fiducials). Änderungen an einem Parameter erzeugen eine eigene Package-Regel.",
    pin_in_paste: "Pin-in-Paste: Öffnung je THT-Pin aus Bohrung (ODB++), Leiterplattendicke, Metallanteil und Blechdicke (IPC-7525, +20 % Meniskus); begrenzt durch max. Größe bzw. Pitch − Mindeststeg.",
    ignore: "Für dieses Package werden keine Pastenöffnungen erzeugt."
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
        pip_max_size: numInput("propPipMax", 0)
    };
}

let ruleUpdateTimeout = null;
function onRuleParamChanged(e) {
    const select = document.getElementById("selectPkgRule");
    // Parameter an einer geerbten Regel geändert → eigene Regel mit dem geerbten Typ
    if (e && e.target && e.target.id !== "selectPkgRule" && select.value === "inherit") {
        select.value = pkgState.effectiveRuleType || "package_group";
        updateRuleFieldVisibility();
    }
    const ruleData = collectRuleFromInputs();
    const target = { ...(pkgState.ruleTarget || { kind: "package", id: pkgState.activePkgName }) };

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
    if (pkgState.pinBarMode === "all") {
        // Alle Pins: erst mit "Anpassen" übernehmen
        document.getElementById("btnSavePadOverride")?.classList.add("btn-pulse");
        return;
    }
    if (!pkgState.activePackageData || pkgState.selectedPin === null) return;
    const pinNum = pkgState.selectedPin;
    const ruleType = document.getElementById("selectPinRule")?.value || "inherit";
    const w = parseFloat(document.getElementById("inputPadCustomW")?.value);
    const h = parseFloat(document.getElementById("inputPadCustomH")?.value);
    const red = parseFloat(document.getElementById("inputPadCustomUndersize")?.value) || 0;

    if (isNaN(w) || isNaN(h) || w <= 0 || h <= 0) return;

    const pkgName = pkgState.activePkgName;

    if (!pkgState.activePackageData.pad_overrides) {
        pkgState.activePackageData.pad_overrides = {};
    }
    pkgState.activePackageData.pad_overrides[pinNum] = {
        rule_type: ruleType,
        w: w,
        h: h,
        undersize_w: red,
        undersize_h: red
    };

    if (pkgState.activePackageData.paste) {
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
            const payload = {
                rule_type: ruleType,
                w: w,
                h: h,
                undersize_w: red,
                undersize_h: red
            };
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
                renderPkgPreview();
            }
        } catch (e) {
            console.error("Fehler beim Live-Sync des Pin-Overrides:", e);
        }
    }, 40);
}

// -------------------------------------------------------------
// Package Canvas Renderer (PCB Investigator Style)
// -------------------------------------------------------------
function renderPkgPreview() {
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

    const data = pkgState.activePackageData;

    // 1. Koordinatenachsen / Nullpunkt
    pkgCtx.strokeStyle = "rgba(255, 255, 255, 0.12)";
    pkgCtx.lineWidth = 1 / pkgState.zoom;
    pkgCtx.beginPath();
    pkgCtx.moveTo(-10, 0); pkgCtx.lineTo(10, 0);
    pkgCtx.moveTo(0, -10); pkgCtx.lineTo(0, 10);
    pkgCtx.stroke();

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
            const isSelected = pkgState.selectedPin === p.pin;
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
        pkgCtx.strokeStyle = "#f8fafc";
        pkgCtx.fillStyle = "rgba(15, 23, 42, 0.85)";
        pkgCtx.lineWidth = 1.2 / pkgState.zoom;
        for (const p of data.pads) {
            if (!p.drill) continue;
            const r = p.drill / 2;
            pkgCtx.beginPath();
            pkgCtx.arc(p.cx, p.cy, r, 0, Math.PI * 2);
            pkgCtx.fill();
            pkgCtx.stroke();
            pkgCtx.beginPath();
            pkgCtx.moveTo(p.cx - r * 0.6, p.cy); pkgCtx.lineTo(p.cx + r * 0.6, p.cy);
            pkgCtx.moveTo(p.cx, p.cy - r * 0.6); pkgCtx.lineTo(p.cx, p.cy + r * 0.6);
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

    // 6. Pin Labels (falls aktiviert)
    if (pkgState.showPinLabels && data.pads) {
        pkgCtx.fillStyle = "#ffffff";
        pkgCtx.font = `${Math.max(10, 14 / pkgState.zoom)}px monospace`;
        pkgCtx.textAlign = "center";
        pkgCtx.textBaseline = "middle";
        for (const p of data.pads) {
            pkgCtx.save();
            pkgCtx.translate(p.cx, p.cy);
            pkgCtx.scale(1, -1); // Text wieder aufrecht
            pkgCtx.fillText(p.pin, 0, 0);
            pkgCtx.restore();
        }
    }

    pkgCtx.restore();

    // Measurements (Package View)
    drawMeasurementLayer(pkgCtx, "package", pkgWorldToScreen, pkgState.zoom);

    // Info-Layer: AR / Fläche / Volumen je Öffnung
    renderPasteInfo(data);

    // Dynamic CAD Scale Bar (Maßstab)
    drawCanvasScaleBar(pkgCtx, w, h, pkgState.zoom, 14, 38);
}

// Pin-Editor im Modus "Alle Pins": Regel/Größe/Reduktion für das ganze Package
function openAllPinsBar() {
    const data = pkgState.activePackageData;
    if (!data) return;
    pkgState.selectedPin = null;
    pkgState.pinBarMode = "all";
    const bar = document.getElementById("singlePadBar");
    bar.classList.remove("hidden");
    const pads = data.pads || [];
    document.getElementById("lblPinBarPrefix").textContent = "";
    document.getElementById("lblSelectedPin").textContent = `Alle Pins (${pads.length})`;
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
    inpW.value = cw ?? "";
    inpH.value = ch ?? "";
    inpW.placeholder = cw === null ? "versch." : "B (mm)";
    inpH.placeholder = ch === null ? "versch." : "H (mm)";
    const undersizeInput = document.getElementById("inputPadCustomUndersize");
    if (undersizeInput) undersizeInput.value = common(ovs.map(o => o.undersize_w)) ?? data.rule?.undersize_w ?? 0.025;
    renderPkgPreview();
}

// Sammel-Übersteuerung für alle Pins (matchPin: Breite/Höhe nur auf gleich große Pads übertragen)
async function applyPinsBulk(override, w, h, matchPin = null) {
    const pkgName = pkgState.activePkgName;
    if (!pkgName) return null;
    document.getElementById("btnSavePadOverride")?.classList.remove("btn-pulse");
    const res = await fetch(`/api/package/${encodeURIComponent(pkgName)}/pins/override`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ override, w: isNaN(w) ? null : w, h: isNaN(h) ? null : h, match_pin: matchPin })
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

function readPinBarValues() {
    return {
        ruleType: document.getElementById("selectPinRule")?.value || "inherit",
        w: parseFloat(document.getElementById("inputPadCustomW")?.value),
        h: parseFloat(document.getElementById("inputPadCustomH")?.value),
        red: parseFloat(document.getElementById("inputPadCustomUndersize")?.value)
    };
}

function selectPkgPin(pinNum, padObj) {
    pkgState.selectedPin = pinNum;
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

    const ruleSelect = document.getElementById("selectPinRule");
    if (ruleSelect) ruleSelect.value = ov?.rule_type || "inherit";

    document.getElementById("inputPadCustomW").value = ov?.w ?? padObj.w;
    document.getElementById("inputPadCustomH").value = ov?.h ?? padObj.h;
    const undersizeInput = document.getElementById("inputPadCustomUndersize");
    if (undersizeInput) undersizeInput.value = ov?.undersize_w ?? pkgState.activePackageData?.rule?.undersize_w ?? 0.025;

    renderPkgPreview();
}

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
        "propCornerRadius", "propUndersize", "propHouseCut", "propPitchRed", "propLengthUse", "propGapRatio", "propLengthComp", "propOutwardShift",
        "propOffsetMoveAway", "propOffsetX", "propOffsetY", "propBridgeX", "propBridgeY", "propBridgeWidth",
        "propToeHeelShift", "propChamferSize", "propSizeW", "propSizeH", "propDonutInner",
        "propBgaThreshold", "propChipThreshold", "propInvertHouse", "propMiniChip",
        "propMinArrayField", "propArrayDistance", "propUtilArray", "propUtilHouse",
        "propPipPcb", "propPipMetal", "propPipClearance", "propPipMax", "propPipShape"
    ].forEach(id => {
        const el = document.getElementById(id);
        if (el) {
            el.addEventListener("input", onRuleParamChanged);
            el.addEventListener("change", onRuleParamChanged);
        }
    });
    document.getElementById("propRotate45")?.addEventListener("change", onRuleParamChanged);
    document.getElementById("propPinMinusMask")?.addEventListener("change", onRuleParamChanged);
    document.getElementById("propChamferMode")?.addEventListener("change", onRuleParamChanged);
    document.getElementById("propTryDirection")?.addEventListener("change", onRuleParamChanged);

    // Auto-Klassifizierung
    async function triggerAutoClassify(btnElement) {
        const orig = btnElement ? btnElement.innerHTML : "";
        if (btnElement) {
            btnElement.innerHTML = `<span>⏳</span> Klassifiziere...`;
            btnElement.disabled = true;
        }
        try {
            const res = await fetch("/api/auto_classify", { method: "POST" });
            const data = await res.json();
            if (data.status === "ok") {
                const c = data.classification;
                showToast(`⚡ Auto-Klassifizierung abgeschlossen! ${c.chips} Chips, ${c.soic} SOIC, ${c.qfn} QFN, ${c.bga} BGA, ${c.tht} THT`);
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
        "inputPadCustomW", "inputPadCustomH", "inputPadCustomUndersize"
    ].forEach(id => {
        const el = document.getElementById(id);
        if (el) {
            el.addEventListener("input", onSinglePinParamChanged);
            el.addEventListener("change", onSinglePinParamChanged);
        }
    });
    document.getElementById("selectPinRule")?.addEventListener("change", onSinglePinParamChanged);

    // Measurement Tool in Stencil Generator
    document.getElementById("btnMeasurePkg")?.addEventListener("click", () => toggleMeasureTool());
    document.getElementById("btnCancelMeasurePkg")?.addEventListener("click", () => toggleMeasureTool(false));
    document.getElementById("btnClearMeasurementsPkg")?.addEventListener("click", clearMeasurements);
    document.getElementById("btnClearMeasurePkg")?.addEventListener("click", clearMeasurements);

    // Package 90° drehen
    document.getElementById("btnRotatePkg")?.addEventListener("click", () => {
        pkgState.rotation = ((pkgState.rotation || 0) + 90) % 360;
        showToast(`⟳ Package gedreht: ${pkgState.rotation}°`);
        renderPkgPreview();
    });

    // Layer Toggles
    document.getElementById("chkPrevMask").addEventListener("change", e => { pkgState.showMask = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevCopper").addEventListener("change", e => { pkgState.showCopper = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevPaste").addEventListener("change", e => { pkgState.showPaste = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevComponent").addEventListener("change", e => { pkgState.showComponent = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevRuler").addEventListener("change", e => { pkgState.showRuler = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevPinLabels").addEventListener("change", e => { pkgState.showPinLabels = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevDrills")?.addEventListener("change", e => { pkgState.showDrills = e.target.checked; renderPkgPreview(); });
    document.getElementById("chkPrevInfo")?.addEventListener("change", e => { pkgState.showInfo = e.target.checked; renderPkgPreview(); });
    setupRuleTargets();

    // Generate Stencil Pads Button
    document.getElementById("btnGenerateStencilPads").addEventListener("click", async () => {
        const target = pkgState.ruleTarget || { kind: "package", id: pkgState.activePkgName };
        if (!target.id) return;
        const btn = document.getElementById("btnGenerateStencilPads");
        const originalText = btn.innerHTML;
        btn.innerHTML = `<span>⏳</span> Generiere...`;
        btn.disabled = true;

        try {
            const ruleData = collectRuleFromInputs();
            const url = target.kind === "package"
                ? `/api/package/${encodeURIComponent(target.id)}/generate`
                : `/api/${target.kind}/${encodeURIComponent(target.id)}/generate`;
            const res = await fetch(url, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: target.kind === "package" ? JSON.stringify(ruleData) : "{}"
            });
            const data = await res.json();
            btn.innerHTML = `<span>✓</span> ${data.modified_count} Pads aktualisiert!`;
            setTimeout(() => {
                btn.innerHTML = originalText;
                btn.disabled = false;
            }, 2500);

            // Aktualisiere Board-Aperturen im Hintergrund
            await refreshApertureData();
        } catch (e) {
            console.error("Fehler bei Generate Stencil Pads:", e);
            btn.innerHTML = originalText;
            btn.disabled = false;
        }
    });

    // Customer Library Actions
    document.getElementById("selectCustomerLib").addEventListener("change", async (e) => {
        const cust = e.target.value;
        const res = await fetch("/api/library/load", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ customer: cust })
        });
        const data = await res.json();
        if (data.status === "ok") {
            await loadPackagesList();
            selectPackage(pkgState.activePkgName);
        }
    });

    document.getElementById("btnSaveCurrentLib").addEventListener("click", async () => {
        const cust = pkgState.currentCustomer;
        const res = await fetch("/api/library/save", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ customer: cust })
        });
        const data = await res.json();
        alert(`Kunden-Bibliothek "${cust}" erfolgreich gespeichert!`);
    });

    // PCB-Investigator-Kundenbibliothek (nur Desktop: Dateidialoge)
    if (!window.__TAURI__) document.querySelectorAll(".desktop-only").forEach(el => el.classList.add("hidden"));
    document.getElementById("btnPcbiImport")?.addEventListener("click", importPcbiLibrary);
    document.getElementById("btnPcbiExport")?.addEventListener("click", async () => {
        const res = await fetch("/api/pcbi/export", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
        const d = await res.json();
        if (d.status === "cancelled") return;
        if (!res.ok || d.status !== "ok") { showToast(`❌ ${d.message || d.error || "Export fehlgeschlagen"}`); return; }
        let msg = `Gespeichert: ${d.path}

${d.packages} Package-Regeln, ${d.pin_rules} Pin-Regeln, ${d.groups} Gruppen, ${d.components} Bauteilregeln.`;
        if (d.skipped?.length) msg += `

Ohne PCB-I-Entsprechung (nicht exportiert):
• ${d.skipped.slice(0, 12).join("\n• ")}${d.skipped.length > 12 ? `
… und ${d.skipped.length - 12} weitere` : ""}`;
        await askChoice("PCB-I-Bibliothek exportiert", msg, [{ label: "OK", value: "ok", cls: "btn-primary" }]);
    });
    document.getElementById("btnAddMissingPaste")?.addEventListener("click", () => offerMissingPaste(true));

    const newLibModal = document.getElementById("newLibModal");
    document.getElementById("btnOpenNewLibModal").addEventListener("click", () => {
        newLibModal.classList.remove("hidden");
    });
    document.getElementById("btnCancelNewLib").addEventListener("click", () => {
        newLibModal.classList.add("hidden");
    });
    document.getElementById("btnConfirmNewLib").addEventListener("click", async () => {
        const name = document.getElementById("inputNewCustName").value.trim();
        if (!name) return;
        await fetch("/api/library/save", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ customer: name })
        });
        newLibModal.classList.add("hidden");
        await loadLibraries();
        document.getElementById("selectCustomerLib").value = name;
        pkgState.currentCustomer = name;
        alert(`Neue Bibliothek "${name}" erstellt!`);
    });

    // Pin Selector Dropdown
    const selectPinDropdown = document.getElementById("selectPinDropdown");
    if (selectPinDropdown) {
        selectPinDropdown.addEventListener("change", (e) => {
            const val = e.target.value;
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
            const newName = prompt(`Neuen Namen für Package "${oldName}" eingeben:`, oldName);
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
                    showToast(`Package umbenannt in "${newName.trim()}"`);
                } else {
                    alert(data.message || "Fehler beim Umbenennen");
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
            if (pkgState.pinBarMode === "all") {
                const v = readPinBarValues();
                const override = { rule_type: v.ruleType };
                if (!isNaN(v.red)) { override.undersize_w = v.red; override.undersize_h = v.red; }
                const data = await applyPinsBulk(override, v.w, v.h);
                if (data?.status === "ok") showToast(`Alle ${data.pins} Pins angepasst`);
                await refreshApertureData();
                return;
            }
            if (!pkgState.activePkgName || pkgState.selectedPin === null) return;
            const pinNum = pkgState.selectedPin;
            const ruleType = document.getElementById("selectPinRule").value;
            const w = parseFloat(document.getElementById("inputPadCustomW").value);
            const h = parseFloat(document.getElementById("inputPadCustomH").value);
            const red = parseFloat(document.getElementById("inputPadCustomUndersize").value) || 0.025;

            const payload = {
                rule_type: ruleType,
                w: w,
                h: h,
                undersize_w: red,
                undersize_h: red
            };

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
                    showToast(`Pin ${pinNum} erfolgreich angepasst!`);
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
        const override = { rule_type: v.ruleType };
        if (!isNaN(v.red)) { override.undersize_w = v.red; override.undersize_h = v.red; }
        const data = await applyPinsBulk(override, v.w, v.h, pkgState.selectedPin);
        if (data?.status === "ok") showToast(`Einstellungen von Pin ${pkgState.selectedPin} auf ${data.pins} Pins übernommen`);
        await refreshApertureData();
    });

    if (btnResetPadOverride) {
        btnResetPadOverride.addEventListener("click", async () => {
            if (pkgState.pinBarMode === "all" && pkgState.activePkgName) {
                const res = await fetch(`/api/package/${encodeURIComponent(pkgState.activePkgName)}/pins/override`, { method: "DELETE" });
                const data = await res.json();
                if (data.preview) {
                    pkgState.activePackageData = data.preview;
                    populatePinDropdown(data.preview.pads, data.preview.pad_overrides);
                    openAllPinsBar();
                    showToast("Alle Pin-Anpassungen zurückgesetzt");
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
                    showToast(`Pin ${pinNum} auf Standard zurückgesetzt`);
                }
                await refreshApertureData();
            } catch (e) {
                console.error("Fehler beim Reset des Pin-Overrides:", e);
            }
        });
    }

    // Single Pad Override Close
    document.getElementById("btnClosePadBar").addEventListener("click", () => {
        if (pkgState.pinBarMode === "single") {
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
            showToast(`⚡ PiP Overprint (${shape}) auf Apertur #${state.selectedApertureId} angewendet!`);
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
                summary.innerHTML = `🟢 <strong>Perfekt:</strong> Keine Stegunterschreitungen unter ${data.min_bridge.toFixed(2)} mm gefunden! Schablone ist laser-fertigungsgerecht.`;
                summary.style.color = "#4ade80";
            } else {
                summary.innerHTML = `⚠️ <strong>${data.count} Steg-Konflikte gefunden!</strong> Mindestabstand ${data.min_bridge.toFixed(2)} mm wird unterschritten (Kollisionsgefahr beim Laserschneiden).`;
                summary.style.color = "#f87171";
            }
        }

        const tbody = document.getElementById("bridgeViolationsTableBody");
        if (tbody) {
            if (data.count === 0) {
                tbody.innerHTML = `<tr><td colspan="4" style="text-align:center; padding: 16px; color: var(--text-secondary);">Keine Stegunterschreitungen gefunden.</td></tr>`;
            } else {
                tbody.innerHTML = data.violations.map(v => `
                    <tr style="border-bottom: 1px solid rgba(255,255,255,0.06);">
                        <td style="padding: 6px 10px;">
                            <span class="badge ${v.severity === 'CRIT' ? 'crit' : 'warn'}" style="font-size:0.7rem; padding: 2px 6px;">
                                ${v.severity === 'CRIT' ? '🔴 Kritisch' : '🟡 Warn'}
                            </span>
                        </td>
                        <td style="padding: 6px 10px; font-weight: 700; color: ${v.severity === 'CRIT' ? '#f87171' : '#fbbf24'};">
                            ${v.dist.toFixed(3)} mm
                        </td>
                        <td style="padding: 6px 10px; color: var(--text-secondary);">
                            #${v.ap_id_1} (${v.pkg_1}) &harr; #${v.ap_id_2} (${v.pkg_2})
                        </td>
                        <td style="padding: 6px 10px; text-align: right;">
                            <button class="btn btn-xs btn-secondary" onclick="zoomToBridgeViolation(${v.p1[0]}, ${v.p1[1]})">
                                🔍 Zu Stelle
                            </button>
                        </td>
                    </tr>
                `).join("");
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
        const icon = document.getElementById("bridgeWarnIcon");
        if (icon) icon.innerText = "⚠️";
    } else {
        badge.classList.add("hidden");
        const icon = document.getElementById("bridgeWarnIcon");
        if (icon) icon.innerText = "🔬";
    }
}

window.zoomToBridgeViolation = function(x, y) {
    document.getElementById("bridgeModal")?.classList.add("hidden");
    state.zoom = Math.max(state.zoom, 35);
    const rect = container.getBoundingClientRect();
    state.panX = rect.width / 2 - x * state.zoom;
    state.panY = rect.height / 2 - y * state.zoom;
    render();
    showToast(`Fokussiert auf Steg bei X: ${x.toFixed(2)}, Y: ${y.toFixed(2)} mm`);
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
    populateRuleInputs(data.rule);
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
    if (undo) { undo.disabled = !h.can_undo; undo.title = h.can_undo ? `Rückgängig: ${h.undo_label} (Strg+Z)` : "Rückgängig (Strg+Z)"; }
    if (redo) { redo.disabled = !h.can_redo; redo.title = h.can_redo ? `Wiederholen: ${h.redo_label} (Strg+Y)` : "Wiederholen (Strg+Y)"; }
    const lbl = document.getElementById("lblProjectName");
    if (lbl) {
        lbl.textContent = h.project_name || (h.dataset_loaded ? "Unbenanntes Projekt" : "–");
        lbl.title = h.project_path || "Noch nicht gespeichert";
        lbl.classList.toggle("dirty", Boolean(h.dirty));
    }
    const base = document.body.classList.contains("view-package-only") ? "Stencil Generator" : "SMT StencilCraft";
    document.title = `${h.dirty ? "● " : ""}${h.project_name ? h.project_name + " – " : ""}${base}`;
}

async function refreshHistory() {
    try {
        const r = await fetch("/api/history");
        updateHistoryUI(await r.json());
    } catch (e) { /* ignorieren */ }
}

// Auswahl-Dialog: liefert den "value" des geklickten Buttons (ESC/Hintergrund = "cancel")
// Bauteilpins ohne Kundenpaste: PCB Investigator erzeugt dort Öffnungen aus dem Kupfer
async function offerMissingPaste(explicit) {
    const m = await (await fetch("/api/paste/missing")).json();
    if (!m.count) {
        if (explicit) showToast("Alle Bauteilpins haben eine Pastenöffnung.");
        return 0;
    }
    const top = Object.entries(m.packages || {}).sort((a, b) => b[1] - a[1]).slice(0, 8).map(([p, n]) => `${p}: ${n}`).join(", ");
    const r = await askChoice("Pins ohne Kundenpaste",
        `${m.count} Bauteilpins haben in der Pastenlage des Kunden keine Öffnung (${top}).

` +
        "PCB Investigator erzeugt dort bei \"Package Group\" Öffnungen aus dem Kupfer. Ergänzen? Die Regeln entscheiden danach je Package (\"Take Existing\"/\"Ignore\" → keine Öffnung). Rückgängig mit Strg+Z.",
        [{ label: "Nicht ergänzen", value: "no", cls: "btn-ghost" }, { label: `${m.count} ergänzen`, value: "yes", cls: "btn-primary" }]);
    if (r !== "yes") return 0;
    const d = await (await fetch("/api/paste/add_missing", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" })).json();
    await refreshApertureData();
    showToast(`✓ ${d.added} Öffnungen aus dem Kupfer ergänzt`);
    return d.added;
}

async function importPcbiLibrary() {
    const cust = pkgState.currentCustomer || "";
    const how = await askChoice("PCB-Investigator-Kundenbibliothek",
        "Regeln (Packages, Pins, Gruppen) aus einer PCB-I-Datei ruleTreeFile_*.xml auf den Datensatz anwenden.\n\n" +
        `Zusätzlich in die StencilCraft-Bibliothek "${cust || "Standard"}" übernehmen (inkl. Packages, die in diesem Job nicht vorkommen)?`,
        [{ label: "Abbrechen", value: "cancel", cls: "btn-ghost" },
         { label: "Nur anwenden", value: "apply" },
         { label: "Anwenden + speichern", value: "save", cls: "btn-primary" }]);
    if (how === "cancel") return;
    const res = await fetch("/api/pcbi/import", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ customer: how === "save" ? (cust || "Standard") : "", generate: true })
    });
    const d = await res.json();
    if (d.status === "cancelled") return;
    if (!res.ok || d.status !== "ok") { showToast(`❌ ${d.message || d.error || "Import fehlgeschlagen"}`); return; }
    await loadPackagesList();
    await refreshApertureData();
    const missing = d.missing_in_library || [];
    let msg = `${d.matched.length} von ${d.library_packages} Bibliotheks-Packages im Datensatz gefunden, ${d.pin_rules} Pin-Regeln, Gruppen: ${(d.group_rules || []).join(", ")}.`;
    msg += `
${d.modified_count ?? 0} Öffnungen neu berechnet.`;
    if (missing.length) msg += `

Ohne Bibliothekseintrag (${missing.length}) – erben von ihrer Gruppe:
${missing.slice(0, 15).map(p => p.name).join(", ")}${missing.length > 15 ? " …" : ""}`;
    if (d.not_in_dataset?.length) msg += `

In der Bibliothek, aber nicht im Job: ${d.not_in_dataset.length}`;
    const unknown = Object.keys(d.unknown_rules || {});
    if (unknown.length) msg += `

⚠ Unbekannte PCB-I-Skripte (als „Original übernehmen“ geführt): ${unknown.join(", ")}`;
    if (d.warnings?.length) msg += `

⚠ ${d.warnings.join("\n⚠ ")}`;
    if (d.library_file) msg += `

Gespeichert in: ${d.library_file}`;
    await askChoice("PCB-I-Bibliothek übernommen", msg, [{ label: "OK", value: "ok", cls: "btn-primary" }]);
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
    const r = await askChoice("Ungespeicherte Änderungen",
        `Das Projekt enthält ungespeicherte Änderungen.\nMöchten Sie speichern, bevor Sie ${actionText}?`,
        [{ label: "Abbrechen", value: "cancel", cls: "btn-ghost" },
         { label: "Verwerfen", value: "discard", cls: "btn-danger" },
         { label: "Speichern", value: "save", cls: "btn-primary" }]);
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
    if (d.status === "ok") { showToast(`💾 Projekt gespeichert: ${d.path}`); return true; }
    if (d.status !== "cancelled") showToast(`❌ Speichern fehlgeschlagen: ${d.message}`);
    return false;
}
window.saveProject = saveProject;

async function reloadAllViews(full = false) {
    if (full) await loadLayers();
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
    if (d.status === "ok") { showToast(`↶ Rückgängig: ${d.label}`, 2000); await reloadAllViews(); }
}

async function doRedo() {
    const d = await (await fetch("/api/redo", { method: "POST" })).json();
    if (d.status === "ok") { showToast(`↷ Wiederholt: ${d.label}`, 2000); await reloadAllViews(); }
}

async function openProject(path = null, extra = {}) {
    if (!extra.relocate && !extra.recovered && !(await guardUnsaved("ein anderes Projekt öffnen"))) return;
    const res = await fetch("/api/project/open", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path, ...extra })
    });
    const d = await res.json();
    if (d.status === "missing_dataset") {
        const r = await askChoice("Datensatz nicht gefunden",
            `Der Datensatz des Projekts liegt nicht mehr am gespeicherten Ort:\n${(d.dataset_paths || []).join("\n")}\n\nBitte den Datensatz (ODB++ / Gerber) neu auswählen.`,
            [{ label: "Abbrechen", value: "cancel", cls: "btn-ghost" }, { label: "Datensatz auswählen…", value: "relocate", cls: "btn-primary" }]);
        if (r === "relocate") return openProject(d.path, { relocate: true });
        return;
    }
    if (d.status === "ok") {
        await reloadAllViews(true);
        showToast(extra.recovered ? "♻️ Sitzung wiederhergestellt" : `📂 Projekt geöffnet: ${d.path}`);
        if (d.warnings?.length) setTimeout(() => showToast(`⚠️ ${d.warnings.join(" ")}`, 6000), 1200);
    } else if (d.status !== "cancelled") {
        showToast(`❌ Projekt konnte nicht geöffnet werden: ${d.message}`, 6000);
    }
}

async function renderRecentProjects() {
    const box = document.getElementById("recentProjectsList");
    if (!box) return;
    const d = await (await fetch("/api/project/recent")).json();
    box.innerHTML = "";
    if (!d.projects?.length) { box.innerHTML = '<div class="dropdown-empty">Noch keine Projekte</div>'; return; }
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
        const r = await askChoice("Sitzung wiederherstellen?",
            `Es gibt eine automatische Sicherung mit ungespeicherten Änderungen:\n${info.dataset || "Unbenannt"}\n\nMöchten Sie diese wiederherstellen?`,
            [{ label: "Verwerfen", value: "discard", cls: "btn-ghost" }, { label: "Wiederherstellen", value: "restore", cls: "btn-primary" }]);
        if (r === "restore") {
            await openProject(info.path, { recovered: true });
            // Sicherung ist nun Teil dieser Sitzung (eigene Sicherung übernimmt)
            await fetch("/api/project/discard_recovery", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path: info.path }) });
        } else if (r === "discard") {
            await fetch("/api/project/discard_recovery", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path: info.path }) });
        }
    } catch (e) { /* ohne Sicherung weiter */ }
}

// Desktop: Fenster schließen mit ungespeicherten Änderungen
window.scHandleCloseRequest = async () => {
    if (await guardUnsaved("die Anwendung beenden")) {
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
            const btn = document.querySelector(`#sideSwitch button[data-side="${sd.side}"]`);
            if (!btn) continue;
            btn.classList.toggle("active", sd.active);
            const usable = sd.loaded || sd.available;
            btn.disabled = !usable && !sd.active;
            const cnt = document.getElementById(sd.side === "top" ? "cntSideTop" : "cntSideBot");
            if (cnt) cnt.textContent = usable ? `(${sd.apertures})` : "";
            btn.title = sd.loaded ? `${sd.side.toUpperCase()}: ${sd.apertures} Pastenöffnungen`
                : sd.available ? `${sd.side.toUpperCase()}: keine Kundenpaste – Öffnungen können aus dem Kupfer ergänzt werden`
                : `${sd.side.toUpperCase()}: keine Daten für diese Seite`;
        }
        state.side = d.active;
    } catch (e) { /* Web-Version ohne Seitenwechsel */ }
}

async function switchSide(side) {
    const res = await fetch("/api/side", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ side }) });
    if (!res.ok) return;
    setPackageHighlight(null);
    state.selectedApertureId = null;
    state.selectedAperture = null;
    await loadLayers();
    await reloadAllViews(false);
    await refreshSides();
    showToast(`Seite ${side.toUpperCase()} aktiv`, 1800);
    // Seite ohne Kundenpaste: Öffnungen aus dem Kupfer anbieten
    const sd = (await (await fetch("/api/sides")).json()).sides.find(x => x.side === side);
    if (sd && !sd.loaded) await offerMissingPaste(true);
}
window.switchSide = switchSide;

document.querySelectorAll("#sideSwitch button").forEach(b => b.addEventListener("click", () => {
    if (!b.classList.contains("active")) switchSide(b.dataset.side);
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
        if (o.zones) lines.push(`${name(sd)}_zonen.gbr (falls Zonen vorhanden)`);
    }
    if (o.outline) lines.push(`${name("Kontur")}.gbr`);
    if (o.report) lines.push(`${exportState.job}_Pruefbericht_${today}.html`);
    document.getElementById("expPreview").innerHTML = lines.map(l => `<div>${l}</div>`).join("") || "–";
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
        document.getElementById("expSides").value = st.sides;
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
    const openPath = (p) => p && window.__TAURI__?.core.invoke("open_path", { path: p }).catch(e => showToast(`❌ ${e}`));
    document.getElementById("btnExpOpenDir")?.addEventListener("click", () => openPath(exportState.lastDir));
    document.getElementById("btnExpOpenReport")?.addEventListener("click", () => openPath(exportState.lastReport));
    document.getElementById("btnExpRun")?.addEventListener("click", async () => {
        const btn = document.getElementById("btnExpRun");
        btn.disabled = true;
        try {
            const res = await fetch("/api/export", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ options: exportOptionsFromForm() }) });
            const d = await res.json();
            if (d.status !== "ok") { showToast(`❌ Export fehlgeschlagen: ${d.message}`, 6000); return; }
            exportState.lastDir = d.export_dir;
            exportState.lastReport = d.report_file;
            const icon = { stencil: "🟪", zones: "🟧", outline: "⬜", report: "📄" };
            document.getElementById("exportMsg").textContent = d.message;
            document.getElementById("exportFileList").innerHTML = d.files.map(f => `<div>${icon[f.kind] || "📄"} ${f.path}${f.count !== undefined ? ` (${f.count})` : ""}</div>`).join("");
            document.getElementById("exportForm").classList.add("hidden");
            document.getElementById("exportResult").classList.remove("hidden");
            btn.classList.add("hidden");
            const desktop = Boolean(window.__TAURI__);
            document.getElementById("btnExpOpenDir").classList.toggle("hidden", !desktop);
            document.getElementById("btnExpOpenReport").classList.toggle("hidden", !desktop || !d.report_file);
            showToast("💾 Export abgeschlossen");
        } finally {
            btn.disabled = false;
        }
    });
}


// -------------------------------------------------------------
// Regelziele: Gruppe / Zone / Bauteil (PCB Investigator: Package-, Zone-, Component-Rules)
// -------------------------------------------------------------
const GROUP_LABELS = { SMD: "SMD Packages", THT: "THT Packages", Other: "Other Packages", Fiducial: "Fiducials", Library: "Bibliothek" };

function updateRuleSourceBanner() {
    const banner = document.getElementById("ruleSourceBanner");
    const text = document.getElementById("ruleSourceText");
    const btn = document.getElementById("btnRemoveTargetRule");
    if (!banner) return;
    const t = pkgState.ruleTarget || { kind: "package" };
    banner.classList.remove("hidden", "inherited", "own");
    btn.classList.add("hidden");
    const inheritOpt = document.querySelector('#selectPkgRule option[value="inherit"]');
    if (inheritOpt) inheritOpt.disabled = t.kind === "group";
    if (t.kind === "package") {
        const d = pkgState.activePackageData;
        if (d?.rule?.rule_type === "inherit") {
            banner.classList.add("inherited");
            text.textContent = `↳ Regel geerbt von ${d.rule_source} – Parameter ändern erzeugt eine eigene Package-Regel`;
        } else {
            banner.classList.add("own");
            text.textContent = `● Eigene Package-Regel (${d?.rule?.rule_type || "-"}) – „Inherit“ wählen, um die Gruppenregel zu verwenden`;
        }
    } else if (t.kind === "group") {
        text.textContent = `Gruppenregel „${GROUP_LABELS[t.id] || t.id}“ – gilt für alle Packages dieser Gruppe mit „Inherit“`;
    } else if (t.kind === "component") {
        const c = pkgState.activeComponent;
        if (c?.has_rule) {
            banner.classList.add("own");
            text.textContent = `● Bauteilregel ${t.id} (überschreibt Zone, Package und Gruppe)`;
            btn.classList.remove("hidden");
        } else {
            banner.classList.add("inherited");
            text.textContent = `↳ ${t.id} nutzt ${c?.rule_source || "-"} – Parameter ändern erzeugt eine Bauteilregel`;
        }
    } else if (t.kind === "zone") {
        const z = (state.zones || []).find(x => x.id === t.id);
        if (z?.rule) {
            banner.classList.add("own");
            text.textContent = `● Zonenregel „${z.name}“ (gilt für Öffnungen in der Zone, außer Bauteilregeln)`;
            btn.classList.remove("hidden");
        } else {
            banner.classList.add("inherited");
            text.textContent = `Zone „${z?.name || t.id}“ hat noch keine Regel – Parameter ändern legt sie an`;
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
    return [`Gruppenregel: ${GROUP_LABELS[key] || key}`,
            g ? `${g.inheriting} von ${g.packages} Packages erben diese Regel · ${g.pads} Pads auf der aktiven Seite` : "",
            "„Generate Stencil Pads“ wendet sie auf alle erbenden Pads an."];
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
    box.innerHTML = zones.length ? "" : '<div class="dropdown-empty" style="padding:12px">Keine Zonen auf dieser Seite. Regelzone im Board aufziehen oder Stufenzone anlegen.</div>';
    for (const z of zones) {
        const el = document.createElement("div");
        const active = pkgState.ruleTarget?.kind === "zone" && pkgState.ruleTarget.id === z.id;
        el.className = `list-item ${active ? "active" : ""}`;
        el.innerHTML = `<div class="li-main">${z.name}<div class="li-sub">${z.width.toFixed(1)} × ${z.height.toFixed(1)} mm</div></div>` +
            (z.step ? `<span class="li-badge step">${Math.round(z.thickness * 1000)} µm</span>` : "") +
            (z.rule ? `<span class="li-badge rule">${z.rule.rule_type}</span>` : '<span class="li-badge">keine Regel</span>');
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
    drawPkgMessage([`Zone: ${z.name}`, `${n} Pads liegen in der Zone${z.step ? ` · Stufe ${Math.round(z.thickness * 1000)} µm` : " · reine Regelzone"}`, "„Generate Stencil Pads“ wendet die Zonenregel an."]);
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
    box.innerHTML = list.length ? "" : '<div class="dropdown-empty" style="padding:12px">Keine Bauteile (nur bei ODB++ mit Bauteildaten).</div>';
    for (const c of list.slice(0, 800)) {
        const el = document.createElement("div");
        const active = pkgState.ruleTarget?.kind === "component" && pkgState.ruleTarget.id === c.refdes;
        el.className = `list-item ${active ? "active" : ""}`;
        el.innerHTML = `<div class="li-main"><b>${c.refdes}</b> <span class="li-sub">${c.package}</span></div><span class="li-badge">${c.pads} Pads</span>${c.has_rule ? '<span class="li-badge rule">eigene Regel</span>' : ""}`;
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
            showToast("Regelzone im Board-Fenster aufziehen (Werkzeug „Zone“, Häkchen „Stufenzone“ entfernen).", 5000);
            return;
        }
        state.drawRuleZone = true;
        switchWorkspace("board");
        startDrawingZone();
        showToast("Rechteck für die Regelzone im Board aufziehen", 3000);
    });
    document.getElementById("btnRemoveTargetRule")?.addEventListener("click", async () => {
        const t = pkgState.ruleTarget;
        if (!t || (t.kind !== "component" && t.kind !== "zone")) return;
        await fetch(`/api/${t.kind}/${encodeURIComponent(t.id)}/rule`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ enabled: false }) });
        await fetch(`/api/${t.kind}/${encodeURIComponent(t.id)}/generate`, { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
        await refreshApertureData();
        if (t.kind === "component") { await loadComponentList(); await selectComponentRule(t.id); }
        else { renderZoneRuleList(); await selectZoneRule(t.id); }
        showToast("Eigene Regel entfernt – Öffnungen neu berechnet");
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
