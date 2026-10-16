// =============================================================
// SMT StencilCraft Desktop - Tastenkombinationen
//   Strg+O  ODB++ / Gerber öffnen
//   Strg+E  Gerber-Export
//   1/2/3   Farbmodus: Originalfarben / IPC-Heatmap / Blechdicke
// (M = Messlineal und ESC/C = Zurücksetzen sind bereits in app.js enthalten)
// =============================================================
window.addEventListener("keydown", (e) => {
    const tag = document.activeElement ? document.activeElement.tagName.toLowerCase() : "";
    const typing = tag === "input" || tag === "textarea" || tag === "select" || document.activeElement?.isContentEditable;
    const ctrl = e.ctrlKey || e.metaKey;

    if (ctrl && e.key.toLowerCase() === "o") {
        e.preventDefault();
        triggerWindowsFileDialog(true);
        return;
    }
    if (ctrl && e.key.toLowerCase() === "e") {
        e.preventDefault();
        document.getElementById("btnExport")?.click();
        return;
    }
    if (typing || ctrl || e.altKey) return;

    const modes = { "1": "layers", "2": "ipc", "3": "thickness" };
    const mode = modes[e.key];
    if (mode) {
        const sel = document.getElementById("colorModeSelect");
        if (sel) {
            sel.value = mode;
            sel.dispatchEvent(new Event("change"));
        }
    }
});
