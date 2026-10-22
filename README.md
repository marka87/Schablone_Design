# SMT StencilCraft v2.6 – Schablonen-Generator & CAD-Optimierer

> **Leitender CAM- & SMT-Softwareentwickler Release v2.6**  
> **Status:** Produktivbetrieb mit erweitertem DFM-, Geometrie- & Through-Hole Reflow Modul  
> **Ziel:** Vollständige Abbildung und Übertreffen von PCB-Investigator bei maximaler Automatisierung und 0ms Live-Reaktivität.

---

## 🌟 Neu in Version 2.6

### 1. DFM-Prüfung: Schablonenstegbreite (Web / Bridge Check)
- **Mindest-Stegbreite im Schablonenblech:** Standardmäßig 0.10 mm bis 0.12 mm (frei parametrierbar im Header und DFM-Modal).
- **R-Tree Spatial Index & Polygon-Distanzprüfung:** Ultraschnelle Kollisionsprüfung via Shapely und R-Tree. Findet Stegunterschreitungen benachbarter Aperturen in < 20 ms.
- **Farbliche Warn-Hervorhebung:** Kritische Stege werden auf dem Board-Canvas mit rot/gelb gestrichelten Verbindungslinien und Maß-Badges (`! 0.082 mm`) hervorgehoben.
- **Interaktives DFM-Modal:** Tabellarische Auflistung aller Stegkonflikte mit 1-Klick-Fokussierung (`🔍 Zu Stelle`).

### 2. Erweiterte Apertur-Geometrien & Slicing
- **45°-Diagonale Stege (Thermal Pads):** Verhindert das Leerrakeln (*Squeegee Scooping*) bei großen Massepads durch 45° gedrehte Trennstege.
- **Toe- / Heel-Shift (Systematische Pad-Verschiebung):** Parametrierbarer Offset der Apertur relativ zum Pad-Zentrum entlang der Bauteilachse:
  - `> 0`: Toe (nach außen zur Bauteilspitze)
  - `< 0`: Heel (nach innen zur Bauteilferse)
- **Chamfered Rectangle (Eck-Anfasung):** 45°-Abschrägung definierter Ecken mit flexiblen Modi:
  - `all`: Alle 4 Ecken angefast
  - `pin1`: Nur Pin 1 angefast (optische Bauteilorientierung)
  - `toe`: Nur Außenkanten (Toe) angefast
  - `heel`: Nur Innenkanten (Heel) angefast
- **Pin-minus-Mask Operation (SMD):** Automatische Schnittmengenberechnung zwischen Kupferpad und Lötstoppmasken-Freistellung für Solder Mask Defined Pads.

### 3. Pin-in-Paste (PiP / Through-Hole Reflow) Berechnungsmodul
- **Exakte physikalische Volumenberechnung:**
  - $V_{\text{Bohrung\_Netto}} = \left(\frac{\pi \cdot D_{\text{Bohrung}}^2}{4} - A_{\text{Pin}}\right) \cdot H_{\text{PCB}}$
  - $V_{\text{Paste\_Soll}} = \frac{V_{\text{Bohrung\_Netto}} + V_{\text{Meniskus}}}{\text{Metallanteil}}$
  - $A_{\text{Apertur}} = \frac{V_{\text{Paste\_Soll}}}{T_{\text{Stencil}}}$
- **Overprint-Geometrien:**
  - ⬛ **Rechteck-Overprint** mit Lötstoppmasken-Überdruck
  - ⭕ **Donut / Annular Ring** mit zentrierter Lochfreistellung
  - ➕ **Kreuzform (Cross-Shape)** zur Lötstoppmasken-Spreizung
- **1-Klick Zuweisung:** Übertragung der berechneten Overprint-Apertur auf das gewählte THT-Pad auf dem Schablonen-Arbeitslayer.

### 4. Automatische Bauteil-Klassifizierung (Auto-Classifier Engine)
- **1-Klick Heuristik beim Import:**
  - **SMD Chips (0201 - 1206):** 2 Pins, symmetrisch, Pitch < 3.2 mm $\rightarrow$ *HouseBuilder* (Anti-Tombstone) bzw. *Fine-Pitch Rect*.
  - **SOIC / SOP / TSSOP:** Parallele Doppelreihen, Pitch 0.5–1.27 mm $\rightarrow$ *Länglich & Schmal* (Pitch-Schutz gegen Lötbrücken).
  - **QFN / DFN:** 4-seitig umlaufend mit zentralem Exposed Pad $\rightarrow$ Periphere Pads geschmälert + *45° Diagonales Array Slicing* auf dem Thermal Pad.
  - **BGA / CSP:** Matrixförmige runde Pins $\rightarrow$ *10% Flächenreduktion*.
  - **THT / Through-Hole:** Durchkontaktierte Pads $\rightarrow$ Vorbereitung für *Pin-in-Paste*.

### 5. Pastenkorn-Validierung (IPC-7525 5-Ball-Regel)
- **Pastentyp-Auswahl im Header:**
  - **Typ 3:** $45\,\mu\text{m} \implies W_{\min} \ge 0.225\,\text{mm}$
  - **Typ 4:** $38\,\mu\text{m} \implies W_{\min} \ge 0.190\,\text{mm}$
  - **Typ 5:** $25\,\mu\text{m} \implies W_{\min} \ge 0.125\,\text{mm}$
- **DFM-Echtzeitwarnung:** Sofortige optische Warnung im Inspector (`#grainAlert`), wenn die kleinste Öffnungsbreite weniger als 5 Pastenkörner fasst.

---

## 🔒 Die Kernregel: Non-Destruktiver Arbeits-Layer
Alle Berechnungen, Regeln und Modifikationen arbeiten **strikt non-destruktiv** auf dem Arbeitslayer (`self.apertures` bzw. `Schablone TOP/BOT Zwischenspeicher`).
- Die Original-Lagen des Kunden (`self.original_apertures`) bleiben unverändert erhalten.
- Im Lagen-Manager können Original-Kundenlage und Schablonen-Zwischenspeicher per Checkbox gleichzeitig eingeblendet und verglichen werden.
- Exportiert werden saubere, seitengetrennte Gerber-Dateien (`Export/stencil_top.ger`, `Export/stencil_bot.ger`).

---

## 🚀 Schnellstart & Bedienung

1. **App starten:** `start_desktop.bat` (bzw. `stencilcraft-desktop/src-tauri/target/release/stencilcraft-desktop.exe`). Bauen mit `build_desktop.bat`.
2. **Oberfläche bearbeiten:** Quellen liegen in `static/` und `templates/`; `build_desktop.bat` übernimmt sie nach `stencilcraft-desktop/ui/`.
3. **Datensatz laden:**
   - Klick auf `Ordner öffnen` oder `ZIP öffnen` (Windows-Dateidialog) oder Drag & Drop.
4. **Auto-Klassifizierung:**
   - Klick auf `⚡ Auto-Klassifizierung` im Header oder Stencil Generator.
5. **DFM Stegprüfung:**
   - Klick auf `🔬 Stegprüfung` im Header zur Anzeige von Stegverletzungen.
6. **Pin-in-Paste Berechnen:**
   - Pad anklicken $\rightarrow$ `🔌 Pin-in-Paste` öffnen $\rightarrow$ Parameter eingeben $\rightarrow$ `⚡ PiP-Apertur auf Schablone erzeugen`.
7. **Export:**
   - Klick auf `💾 Gerber Export` speichert laserfertige Schablonendaten in den gewählten Export-Ordner.

---

## 🖥️ Desktop-App (Rust / Tauri) – `stencilcraft-desktop/`

Eigenständige Windows-Anwendung ohne Python-Laufzeit. Die Oberfläche (`static/`, `templates/`) wird unverändert übernommen, alle `/api/...`-Aufrufe gehen über die Tauri-Bridge an die native Rust-Engine. Das frühere Python-Backend (Flask) wurde entfernt.

| Pfad | Inhalt |
|---|---|
| `src-tauri/core/src/engine/` | Engine: Regelgeometrie (`rules.rs`), IPC-7525/Stegprüfung/PiP/Kalkulator (`dfm.rs`), Package-Erkennung & Auto-Klassifizierung (`packages.rs`), ODB++ (`odb.rs`), RS-274X Leser/Schreiber (`gerber.rs`), Import (`import.rs`) |
| `src-tauri/core/src/api.rs` | Router aller `/api/...`-Routen der Oberfläche |
| `src-tauri/src/lib.rs` | Tauri-Commands `api_request` und `stage_upload` (Drag & Drop), native Windows-Dialoge |
| `ui/bridge.js`, `ui/desktop.js` | `fetch("/api/...")` → Rust; Tastenkürzel (Strg+O, Strg+E, 1/2/3) |
| `ui/app.js`, `ui/style.css`, `ui/index.html` | Kopie der Web-Oberfläche (wird von `build_desktop.bat` synchronisiert) |

**Bedienung (Auszug):**
- Projekt: `Strg+S` speichern, `Strg+Umschalt+S` speichern unter, Menü „📁 Projekt“ mit „Zuletzt geöffnet“. Projektdateien (`*.stencilproj`) enthalten Datensatzpfad, Parameter, Regeln, Pin-Anpassungen, Zonen und alle Öffnungen; verschobene Datensätze können beim Öffnen neu ausgewählt werden.
- Rückgängig/Wiederholen: `Strg+Z` / `Strg+Y` (schnelle Folgeänderungen an derselben Stelle zählen als ein Schritt).
- Autospeichern jede Minute nach `Autosave/`; nach einem Absturz wird die Wiederherstellung angeboten.
- „⧉ Eigenes Fenster“ öffnet den Stencil Generator separat (z. B. zweiter Monitor); Auswahl und Änderungen werden zwischen beiden Fenstern synchronisiert, das gewählte Package wird im Board orange hervorgehoben.
- TOP/BOT: ODB++ und Gerber-Archive mit beiden Pastenlagen laden beide Seiten; Umschalter in der Werkzeugleiste. Öffnungen, Zonen und Blechdicke gelten je Seite, Package-Regeln für beide.
- Export-Dialog (`Strg+E`): Zielordner, Dateinamen-Muster (`{job}` `{side}` `{dicke}` `{datum}`), Seiten (aktiv/TOP/BOT/beide), Stufenzonen, Platinenkontur, Gerber-X2-Attribute und Prüfbericht (HTML, druckbar/als PDF). Die Optionen werden gemerkt.
- Regelhierarchie wie PCB Investigator: Reiter „Package Rules“ (Gruppenregel per Klick auf den Gruppennamen, Packages mit „Inherit“ erben sie), „Zone Rules“ (Regel für einen Bereich, wahlweise ohne Dickenänderung) und „Component Rules“ (Regel je Bauteil). Rangfolge: Bauteil → Zone → Package (+ Pin-Anpassung) → Gruppe.
- Regel „Pin-in-Paste“: Öffnung je THT-Pin automatisch aus Bohrung, Leiterplattendicke, Metallanteil und Blechdicke (Kreis/Quadrat/Donut, begrenzt durch Pitch − Mindeststeg).
- „Show Info Layer“ im Generator: Area Ratio, Fläche, Volumen je Öffnung und bei Pin-in-Paste die Volumen-Erfüllung in %.
- PCB-Investigator-Kundenbibliothek: „⇩ PCB-I-Bibliothek“ liest `ruleTreeFile_*.xml` (Package-, Pin- und Gruppenregeln: Simple Rectangle Surface, Handle Stencil Creation by Package Group, HouseBuilder, Take Existing, Ignore) und übernimmt sie auf Wunsch in die StencilCraft-Bibliothek, inklusive der Packages, die im aktuellen Job nicht vorkommen. Der Bericht listet Packages ohne Bibliothekseintrag. „⇧ Export PCB-I“ schreibt die aktuellen Regeln im selben Format zurück; der `<Settings>`-Block der importierten Datei bleibt dabei erhalten.
- „Package Group“ rechnet wie PCB Investigator vom Kupferpad der Außenlage aus (minus Undersize, abgerundetes Rechteck; BGA-Pads rund; 2-Pin-Chips bis zur Chip-Schwelle ohne Haus).
- „＋ Fehlende Paste“: Bauteilpins ohne Öffnung in der Kundenpaste (Testpads, Spark-Gaps, …) werden aus dem Kupfer ergänzt; die Regeln entscheiden je Package, ob dort eine Öffnung entsteht.
- Abgleich mit PCB Investigator: `cargo run --release -p stencil_core --example pcbi_compare -- <job.tgz> <ruleTree.xml> <pcbi_top.ger> <pcbi_bot.ger>` vergleicht Öffnung für Öffnung (Lage, Fläche ±5 %) und listet Abweichungen je Package.
- UI-Größe oben rechts (Auto / 80–140 %), Bereiche per Ziehleiste in der Größe veränderbar, Doppelklick klappt sie ein.

**Bauen:** `build_desktop.bat` (synchronisiert die Web-UI, führt die Engine-Tests aus, baut `target\release\stencilcraft-desktop.exe`).
**Starten:** `start_desktop.bat`.
**Tests:** `cargo test -p stencil_core` im Ordner `stencilcraft-desktop/src-tauri`.

Bibliotheken (`Libraries/*.json`, gleiches Format wie die Web-App) und Exporte (`Export/`) liegen im ersten übergeordneten Ordner der EXE, der einen `Libraries`-Ordner enthält; alternativ per Umgebungsvariable `STENCILCRAFT_HOME`.
