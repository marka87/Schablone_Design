"""Schablonendicke-Vorschlag aus Pad-/Paste-Layer (Gerber), rein lokal.

Aufruf:  python schablone.py paste.gbr [faktor]
faktor = Aperturverkleinerung, z.B. 0.9 (Standard 1.0 = Apertur = Pad)
"""
import sys
from collections import Counter
from gerbonara import GerberFile
from gerbonara.utils import MM

DICKEN = [0.08, 0.10, 0.12, 0.125, 0.15, 0.18, 0.20]  # mm, an euren Lieferanten anpassen
AREA_MIN = 0.66    # IPC-7525 Area Ratio (Laser)
ASPECT_MIN = 1.5   # IPC-7525 Aspect Ratio


def max_dicke(w, h):
    """Größte erlaubte Schablonendicke (mm) für eine rechteckige Apertur w x h."""
    return min(min(w, h) / ASPECT_MIN, w * h / (2 * (w + h) * AREA_MIN))


def main(pfad, faktor=1.0):
    pads = Counter()
    for o in GerberFile.open(pfad).objects:
        (x0, y0), (x1, y1) = o.bounding_box(unit=MM)
        w, h = round((x1 - x0) * faktor, 3), round((y1 - y0) * faktor, 3)
        pads[(w, h)] += 1  # ponytail: Bounding-Box statt echter Form (Kreis/Rechteck exakt, Obround leicht pessimistisch)

    grenzen = sorted((max_dicke(w, h), w, h, n) for (w, h), n in pads.items())
    limit = grenzen[0][0]
    ok = [d for d in DICKEN if d <= limit]

    print(f"{sum(pads.values())} Pads, {len(pads)} verschiedene Größen, Faktor {faktor}\n")
    print("engste Aperturen (Größe mm | max. Dicke | Anzahl):")
    for g, w, h, n in grenzen[:5]:
        print(f"  {w:.3f} x {h:.3f} | {g:.3f} | {n}x")
    print(f"\nVorschlag: {ok[-1]:.3f} mm" if ok else
          f"\nKeine Standarddicke passt (Limit {limit:.3f} mm): kleinere Dicke, Step-Down oder Pad-Layout prüfen")
    return limit


if __name__ == "__main__":
    if len(sys.argv) < 2:
        # Selbsttest: 0.5x0.5 -> Area Ratio begrenzt (0.189 mm); 0.3x5.0 -> Aspect begrenzt (0.2 mm)
        assert abs(max_dicke(0.5, 0.5) - 0.1894) < 1e-3
        assert abs(max_dicke(0.3, 5.0) - 0.2) < 1e-3
        print("Selbsttest ok")
    else:
        main(sys.argv[1], float(sys.argv[2]) if len(sys.argv) > 2 else 1.0)
